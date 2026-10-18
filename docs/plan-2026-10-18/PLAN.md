# fbittorrent plan: one daemon, two native surfaces

Date: 2026-10-18. Round: **6 (the owner's answers folded in: one stop rule, the Asupersync fork
accepted)**; round 4 (fresh-read review integrated) and the round-5 verification and prose pass
precede it (§0.7), as do rounds 0 (first draft), 1 (ambition), 2 (architecture and product
reviews) and 3 (consistency and validation; every work item self-contained). Code baseline: branch
`codex/shows-native-engine` at `564fa18` plus its uncommitted working tree (the native engine).
Status: draft for owner review. Review history: `review-log.md` next to this file.

This plan is meant to be implemented by an agent that has not read the research. It restates
every fact it depends on. Provenance markers:

- `path:line` refers to the working tree on 2026-10-18. Line numbers drift; search for the quoted
  symbol if a line no longer matches. Paths under `/usr/share/omarchy/shell/` are the installed
  Omarchy shell (`omarchy-dev 4.0.0.r7025`); paths under `~/.cargo/registry/src/*/` are the pinned
  crates (`ftui-core-0.6.0`, `ftui-widgets-0.6.0`, `ftui-extras-0.6.0`).
- [RC §n] is `docs/plan-2026-10-18/reality-check.md`, [FO §n] is `frontend-options.md`,
  [OR §n] is `omatorrent-research.md`. They hold the evidence; this plan holds the decisions.
  [EE "section"] is `docs/engine-evaluation.md`; [UX] is `docs/tui-ux-review.md`.
- Research clones under the session scratchpad (`qsr/omaqbt-files`, `research/cuciz`,
  `prior-art/*`) are cited as `omaqbt <file>:<line>`, `Sprout <file>:<line>`,
  `Fragments <file>:<line>`, `Transmission <file>:<line>`, `qBittorrent <file>:<line>`. They are
  read-only reference material; nothing in them is executed or copied verbatim.
- "Unverified" marks a claim nobody has checked on this machine. Every unverified claim that a
  work item depends on has a verification step inside that item, and §10.4 lists them all.

How to read it: the owner reads §0 to §2 and §10.3. Implementers read §3 to §9 and the appendices.
Designers read §4 and §5. Work item IDs (`P2-03`) are stable across rounds; an item that is
removed or merged keeps its ID with the note "superseded by …".

---

## 0. Executive summary

### 0.1 The decision

**Keep both surfaces, over a headless daemon.** The same `fbittorrent` binary gains a
`fbittorrent serve` mode that owns the torrent engine, search, RSS, Shows and IMDb data. Clients
are: the Omarchy bar widget with its popup, a new Quickshell dashboard window, the FrankenTUI,
the CLI, and the `magnet:` handler. They all speak one versioned API (newline-delimited JSON over
the existing private Unix socket directory) that sends a snapshot and then pushed deltas. No Rust
GUI toolkit is added.

Why this and not something else, in four grounded points:

1. **The engine cannot leave the TUI today, and that blocks every graphical option.**
   `run_native_app` starts the engine, blocks in `tui::run`, then drops the engine
   (`src/main.rs:786-827`). `fbittorrent tui` refuses to run without a TTY
   (`src/main.rs:452-455`). A dashboard would have to open a terminal to host its engine.
   A daemon is the enabling step whatever frontend wins [FO §0, §4.1].
2. **The split is cheap.** The TUI is already a socket client of its own engine: every read and
   write goes through `qbt::Client` to `ipc::request` (`src/tui/mod.rs:353-375`,
   `src/qbt.rs:295`, `src/ipc.rs:87-122`). The only in-process couplings are three flags the TUI
   polls every 2 s (`src/tui/mod.rs:1699-1702`) and the "on open" RSS and Shows checks in its
   `init` (`src/tui/mod.rs:1664-1666`) [RC §6].
3. **Quickshell is the only way to look like the Omarchy panels the owner admires.** It reuses
   the shell's own `Panel`, `KeyboardPanel`, `PanelKeyCatcher` and `qs.Commons` tokens, and themes
   switch live [FO §2.4, §2.5, §3.4]. Every Rust toolkit would be a third UI technology that has
   to re-skin itself from `colors.toml` [FO §3.1, §3.4].
4. **The TUI stays because it works where the shell does not.** It works over SSH and on a TTY, carries
   the keyboard muscle memory, and is the only UI with automated end-to-end tests (tmux). The
   Quickshell code runs inside the single desktop shell process with no isolation and on a
   pre-1.0 platform [FO §2.7, §2.11]. When the shell breaks, the TUI still works.

The grounded case for dropping the TUI (option "Quickshell only") is thin: the decision matrix
puts it one point behind (77 vs 78 of 95) and it wins only if SSH and TTY reach are worth nothing
[FO §6]. The owner's agent/CLI surface and the test argument above make that unlikely. The plan
keeps the TUI as a first-class client. (Round 0 scheduled a "retirement review"; round 1 removed
it as ceremony, see P7-07. The per-client usage counters it would have read stay, because they
cost one line in `hello` handling and the owner can look at them whenever they like.)

### 0.2 What "optimal" means here

The owner's complaint is not "missing features", it is "the look, feel and features have not
reached OmaTorrent's level". OmaTorrent's level, concretely, is a native Omarchy surface fed by a
daemon [OR §2.1]. This plan is optimal when all of the following are true, and §9's acceptance
criteria are the checks:

| Dimension | Today | Target | Where |
|---|---|---|---|
| Look | A careful TUI prototype [RC §9]; a popup built from shell components but fed by polling | Popup and dashboard built only from shell tokens and components (lint), with the popup captured side by side with the Wi-Fi panel in the same theme and judged by the owner against it (`qs-visual.sh --compare omarchy.network`; our hero, meta, separator and row geometry printed under the pair), every §4.11 state captured in the owner's theme and one light theme; a TUI whose state column reads at 80 columns on a monochrome theme (goldens) | §4, §5, P2-08 |
| Feel | Every action is a process spawn or a 2 s poll; closing a window kills downloads; a magnet click opens a terminal | Sub-second pushed updates; nothing is spawned per tick; downloads outlive windows; a magnet click is silent and notifies | §3 |
| Features | 9 of 36 qBittorrent features present [RC §8]; no peers, limits, recheck, log, notifications | Everything a streaming-first personal client needs (§6), including things qBittorrent does not have: ready-to-watch, show inbox, poster browsing, agent CLI | §6, §2.7 |
| Trust | "0 seeds", `eta: 8640000`, yes-by-default deletes, a widget that resurrects the app | Unknown is absent; every destructive step confirms with Cancel focused; nothing starts the daemon except a write | §2.2, §3.4.9, §4.9 |
| Verifiability | 90 unit tests and one tmux harness; nothing in git | Every work item ships unit tests plus an end-to-end case with JSONL logs; CI on `main`; fixtures shared by Rust and QML | §8.4, §9.1, Appendix E |

### 0.3 Assumed answers to the owner decisions

The research left four decisions open [FO §7.4]; round 1 added four that surfaced while
designing, and round 4 one more. The plan assumes the answers below and is complete under them.
Each is a setting or a scoped choice, so a different answer changes a default, not the
architecture. §10.3 repeats them as questions.

**A1 (answered by the owner, 2026-10-18). One stop rule: the daemon runs while the bar icon is
lit and stops when the owner stops it; an idle net catches what the owner forgets.** The owner's
words: "I don't want running processes. When we close the toolbar widget, it stops, and is done."
Encoded: the torrent icon in the bar is visible exactly while the daemon runs (lit icon = running
process; hidden when stopped, Q2); the icon's power button, `fbittorrent down` and `Q` in the TUI
stop the daemon with everything in it (the popup and the TUI confirm when transfers are active);
closing the TUI or the dashboard window never stops a transfer, dismissing the popup never stops
anything, and the bar widget never keeps the daemon alive (it is not an interactive client; an open
popup only pauses the countdown while it is open). The idle net, `idle_exit_secs = 300`, exits the
daemon by itself
five minutes after the last download or allowed seed ends, so nothing lingers if the owner
forgets; `0` disables it. "Work" is the busy table of §3.2.10 and nowhere else. There is no
`stop_policy` setting: `last-ui` is removed (Q31) and `explicit` is superseded by the power button
plus the idle net (DL-70). Closing a window must still never kill a download ([RC §5] defect 6);
seeding is off by default and torrents stop at completion (`src/engine.rs:306-310`).

**A2. The TUI stays long-term, as a pure API client at its current scope plus the T1 features.**
Rationale: SSH and TTY reach, the tmux test harness, and resilience when the desktop shell
breaks (§0.1 point 4). Its cost becomes UI code only, because every feature lands in the daemon
API first (§2.2 principle 6). T2 and T3 features go to the dashboard first and reach the TUI
when cheap.

**A3. Dashboard surface: a Quickshell `FloatingWindow` owned by a `panel`-kind entry point of
the same plugin. No overlay in v1.** Rationale: the torrent list is a long-lived surface you keep
open on a workspace while watching; an overlay is modal and full-screen and suits short tasks
[FO §2.3, §7.4]. A `panel` kind can live in the same plugin as the bar widget (omaqbt does this),
whereas adding an `overlay` kind to a bar-widget plugin breaks `omarchy-shell shell toggle`
routing [OR §6 gotcha 4]. Verified on this machine: `shell.qml:1161-1172` routes a plugin that
declares `panel` to the panel loader, and `computePanelEntries` (`shell.qml:1356-1372`) accepts
any enabled plugin with a panel kind, where "enabled" includes a bar-layout entry
(`services/PluginRegistry.qml:148-163`; the owner's `~/.config/omarchy/shell.json:38` lists the
plugin only in the bar layout). A quick-add and search palette as an overlay is a later option
(P7-05).

**A4. Search, RSS, Shows and IMDb move into the daemon in P3, before the dashboard views that
use them. QML never shells out per action.** Rationale: a CLI bridge would make the plugin fork
`fbittorrent` processes from inside the desktop shell, which is the polling anti-pattern that
cost ucmz851 its review rounds and that Omarchy's own guide warns about [OR §2.2, §6 gotcha 9;
FO §1.4; `omarchy-src/agents/skills/shell-dev.md` "Keeping state current without polling"].
Moving them also lets RSS refresh and search results survive closing a window. The CLI keeps an
in-process fallback for these read-mostly commands when the daemon is not running, so
`fbittorrent search` still works for agents without starting a torrent engine (§3.7).

**A5 (new). A `magnet:` click adds immediately and notifies; it never opens a window.** The
notification offers "Undo" and "Open window". `handler = "confirm"` instead opens the dashboard's
add sheet with the magnet staged (P3-11). Rationale: the click is the deliberate act (§2.2
principle 2); omaqbt's magnet confirm popup exists because qBittorrent cannot stage, while our
staged add can serve the owners who want a second look [FO §5; omaqbt `Panel.qml:86-99`].

**A6 (new). "Ready to watch" is a first-class state.** The daemon derives, per video file, whether
enough contiguous data from the start exists to start playing, pushes it, notifies once, and every
UI shows it (P5-05). Rationale: sequential plus first/last piece exists for streaming; today no
surface tells the user when streaming can start [RC §3 #2].

**A7 (new). Opt-in clipboard watching, off by default.** When on, the daemon notices a magnet on
the Wayland clipboard and offers to add it (P6-13), the Fragments pattern [FO §5]. Rationale: it
removes the last two keystrokes from "find in browser, download", but reading the clipboard
continuously is a privacy decision the owner makes, not the plan.

**A8 (new). The monochrome theme is the design constraint, not an edge case.** The owner's
active theme has `red = #a4a4a4`, `green = #b6b6b6`, `dark_foreground = #505050`
(`~/.local/state/omarchy/current/theme/colors.toml`). Every state in every surface carries a
glyph and a word; colour is reinforcement. Rationale: [RC §9] defects 1 to 3; §2.2 principle 4.

**A9 (round 4; Q30 answered 2026-10-18). Two defaults the plan runs on until the owner says
otherwise** (Q32, Q33): the popup's key-hint row is hidden at rest and appears from the first key
press, so the resting popup has the Wi-Fi panel's structure (Q32); a `magnet:` click gets the
daemon's "Added" toast with Undo and Open window and a terminal `add` a plain one (Q33). Q30, the
Asupersync rider's "available to" clause, is answered: the owner accepts the risk ("who cares,
continue"); the plan proceeds as written and still draws no legal conclusion (DL-70).

### 0.4 What changes for the user, and what does not

| Changes | Why |
|---|---|
| Clicking a `magnet:` link adds the torrent silently and shows a notification with "Undo" and "Open window". No terminal window opens. | Today `add` opens the TUI through `omarchy-launch-or-focus-tui` (`src/backend.rs:62-86`). |
| The torrent icon in the bar is lit while fbittorrent runs; click its power button and it stops, and is done. Closing the TUI or the dashboard window never stops downloads, and dismissing the popup never stops anything; `fbittorrent down` and `Q` in the TUI stop it too (`Q` confirms when transfers are active). If you forget, the daemon exits by itself five minutes after the last download or allowed seed ends. | A1 (the owner's answer, 2026-10-18). |
| The bar popup updates live (pushed), and reaches the dashboard and the TUI in one key. | One socket in a `service` singleton replaces a `status --json` process per monitor every 2 to 10 s (`plugin/oliver.torrents/Service.qml:80-118`, `Panel.qml:119-122`). |
| A dashboard window (Torrents, Search, Shows, RSS, Settings, Log) in Omarchy's own look. | §4. |
| "Ready to watch" appears on a row as soon as the first minutes of a video are on disk, with a notification whose "Open" starts the player. | A6, P5-05. |
| Completion notifications with "Open" and "Show in folder". | Missing today [RC §8]. |
| Destructive prompts default to No; the popup asks before removing. | [RC §9] defects 5 and 15. |
| Peers, recheck, global and alternative ("turtle") speed limits, a log, later trackers, pieces and speed graphs. | §6. |
| `fbittorrent up` starts the engine without opening a window; `fbittorrent dashboard` and `fbittorrent tui` open UIs; `fbittorrent events --follow` streams what happens for agents. | The daemon separates "engine" from "window". |

| Does not change | Where it is enforced |
|---|---|
| Binary name `fbittorrent`; one binary for daemon, TUI and CLI. | §3.3 |
| Sequential download and first/last piece priority, always on, not toggleable. | `vendor/rqbit/librqbit/src/file_info.rs:14-25`, refusals at `src/engine.rs:648-649`. |
| Seeding off by default: 32 KiB/s upload cap while downloading, stop when complete, per-torrent `seed` override. | `src/qbt.rs:40-44`, `src/engine.rs:833-848`. |
| Downloads go to `~/Downloads`. | `src/engine.rs:403-409` |
| Nothing runs at login. The systemd unit has no `[Install]` section, so enabling it links nothing and it never starts at login. | §3.2.2 |
| The bar widget uses the shell's `Panel` chrome, never a tray icon, and is visible only while fbittorrent runs (a setting can change that, §4.5). | §4.5 |
| Ghostty for the TUI; live theme from the active Omarchy theme. | `src/backend.rs:62-86`, `src/tui/theme.rs:57-79` |
| RSS is feeds and items, no rule engine. Checking never downloads. | §6.2 row "RSS auto-download rules". |
| IMDb cards with rating, blurb and poster, no API key. | `src/imdb.rs:1-4`, §3.7 |
| The documented keys keep their meaning. | Appendix B |
| `triage`, `capabilities`, `robot-docs`, exit codes 0..8 and `--json` stay; changes are additive (contract 1.2.0) except the declared `status` stopped-shape fix (DL-22, Appendix F). | P1-11, Appendix F |
| ProtonVPN or exact-interface binding with a visible waiting state. | §3.2.9 |

### 0.5 Phases, milestones and what each demo shows

| Phase → milestone | What the owner sees at the demo (§9.7) | Release |
|---|---|---|
| P0 → M0 "safe on main" (16 items) | Today's app, now in git with CI, the JSONL test logging and the golden-render harness; the live defects fixed on today's architecture: no relaunch bug, no yes-by-default delete, no startup blocker, no "0 seeds" or `eta: 8640000` | v0.1.1, v0.1.2 |
| P1 → M1a "daemon behind today's UIs" (19 items; installed, not tagged) | `scripts/install.sh`, then a magnet click with every window closed: no terminal, one "Added" toast with Undo and Open window; the TUI's `q` leaves the download running; `fbittorrent status --json` counts down the idle exit; `events --follow` streams `torrent.completed` to an agent; the v0.1 popup still works and still hides when the daemon stops | v0.2.0-pre |
| P2 → **M1, the first demo** (8 items) | The §4 popup: rows pushed over one socket, no process spawned per tick, the add preview, the confirm strip and the power button that stops everything (A1), captured side by side with the Wi-Fi panel in the owner's theme and one light theme | v0.2.0 |
| P3 → M2 (18 items) | Search, RSS, Shows and IMDb run in the daemon; the dashboard window with its six views, the poster grid and show quality; every step of W1, W2 and W8 from the window | v0.3.0 |
| P4 → M3 (11 items) | The TUI as a pure client, split into modules, readable at 80 columns on the monochrome theme over SSH; the 17 UX defects fixed; legacy mode removed | v0.4.0 |
| P5 → M4 (7 items) | Peers and true seed counts, recheck, turtle and global limits, "Ready to watch" that does not cry wolf (`stall_free`), the low-disk banner, in both UIs | v0.5.0 |
| P6 → M5 (13 items) | Trackers, pieces, graphs, statistics, queue control, fast resume, undo, the opt-in clipboard watcher | v0.6.0 |
| P7 → M6 (7 items) | Torznab, move, rename, tracker editing, the quick-add palette, deferred add | as they land |

M1a is the first moment the owner lives with the new lifetime (R5) and makes the one-time upgrade
check of §3.6 by hand. Its chain is closed end to end: the install script exists at P1-04 and runs
`setup --magnet`; an `add` that arrives while the engine slot is still `Starting` waits for the
session instead of failing; exactly one toast is posted, by the daemon, for a browser click; the
v0.1 popup's `online` test survives the 1.2.0 stopped shape (§9.7 "M1a demo script", DL-66). M1
adds the §4 popup. With all windows closed: click a magnet link in the browser. No terminal opens;
a notification says "Added" with "Undo" and "Open window". The bar icon appears and shows download
activity within a second. The popup shows the torrent's progress moving without any `fbittorrent`
process being spawned by the shell. Close the popup and wait: when the download completes, a
notification offers "Open" and "Show in folder". Click the icon's power button and it stops, and is
done; or leave it, and five minutes after the download ends the daemon exits by itself and the icon
disappears. `fbittorrent status` never starts anything. The full demo script is in §9.7.

### 0.6 The plan in numbers, and what is still open

- Work: 99 items (13 L, 44 M, 42 S; weights S=1, M=2, L=4) in eight phases and 8 milestones (M0,
  M1a, M1 to M6): 107 nodes in 22 topological levels, no cycle, no orphan (§9.4, generated from
  §9.3 by script). Two IDs are superseded and kept (P7-07, P7-08). Every item is self-contained to
  the standard of the §9.3 preamble (files, API names, tokens, thresholds, fixture paths, section
  anchors); twelve items were handed alone to fresh agents in rounds 3 and 4 and every gap they
  named is closed in the text.
- Critical path: 32 S-units to M1a and to M1, through P0-01 → P0-02 → P0-04 → M0, then P1-01 →
  P1-02 → P1-19 → P1-05 → P1-06 → P1-08, then either the daemon branch P1-12 → P1-17 → P1-11 →
  P1-15 or the plugin branch P2-01 → P2-02 → P2-03 → P2-05 (they tie at 9); M2 38, M3 46, M4 42,
  M5 48, M6 50 (§9.4).
- Decisions DL-01 to DL-70 (§10.2); risks R1 to R19 (§10.1); every unverified claim with the item
  that verifies it (§10.4).
- Owner questions (§10.3): 31 asked; six answered on 2026-10-18 (Q1 and Q31: one stop rule, no
  `stop_policy`; Q2: the icon is hidden when stopped; Q3 and Q30: the public fork, the rider's risk
  accepted, "who cares, continue"; Q4: no archive, README summaries kept); 25 open, each with the default the plan already runs on. None
  gates work any longer: P0-02 proceeds once the owner creates the repository or says "fallback".
  Q32 (the popup's key-hint row at rest) and Q33 (the toast a terminal `add` gets) change a
  default; the rest shape P2 onwards: Q6 (chord), Q9 to Q11, Q12 to Q20, Q21 to Q29.

### 0.7 Review history at a glance

`review-log.md` holds a verdict per review item for rounds 2 to 4, the self-containment
transcripts verbatim and the script outputs. Round 0 was the first draft.

- Round 1 (ambition): design specifications per surface in Omarchy's own tokens (§4.2 to §4.4,
  §5.5, §5.6); API v1 fully typed (Appendix A) with a 1:1 CLI mapping (Appendix F); the daemon
  lifetime machine (§3.2.7); four features beyond qBittorrent (§2.7); tests that log for every
  item; the dependency graph (§9.4); the one ceremony item removed (P7-07).
- Round 2 (architecture and product/UX reviews): dashboard geometry as arithmetic at every width
  tier; contrast gates the installed themes pass; one state table for three surfaces; `stall_free`;
  disk-full behaviour (P5-07); draining, mid-request loss and background restore written down;
  `events` replayable; P1-02 split (P1-19); nine citations corrected (DL-30 to DL-49).
- Round 3 (consistency and validation reviews): one dim ladder with formulas; `q` closes and `Q`
  quits everywhere; A.1's idempotent list with `instance`; M1a and its demo; the harness skeleton
  before the daemon; cuts of machinery (DL-53 to DL-60); the UI contract files given shapes; every
  §9.3 item rewritten to be self-contained; §9.4 generated by script; eight items sampled with
  fresh agents and a dozen designs settled (DL-61 to DL-65).
- Round 4 (a fresh full read, a 22-step walk of the M1a demo, four more sampled items): the M1a
  chain closed (DL-66); protocol holes filled: a request-less `error` event, `since_seq`,
  `subs: 16`, the closed coalesced set, the line-cap rule, `finished_notify` and a ledger
  generation counter, the client kinds, the bulk `X` shortcut (DL-67); more machinery cut:
  `--test-proto`, `--detach` and its double fork, the timer-wheel runner, `ping`, per-torrent
  rings (DL-68); the key-hint row hidden at rest, sentinels kept in contract-1.x fields, one unit
  rule, `LICENSE` and `NOTICE`, Torznab's caps reconciled (DL-69); P0-02 after P0-01, which makes
  the critical path 32 S-units; Q30 to Q33 added; three citations corrected. The plan is at steady
  state: the round's structural findings were gaps, not redesigns, and the next step is conversion
  to beads, not another review round.
- Round 5 (verification and prose): every ID and section reference checked by script (0
  unresolved), the §0 numbers checked against §9.4 and §10, the mockup widths re-measured; stale
  statements aligned with the canonical text (§3.2.11's start limit, the "Added" rows of §3.8 and
  §6.3, §4.18's unknown marker, §10.5's round number) and wording tightened. No decision, number or
  structure changed.
- Round 6 (the owner's answers, 2026-10-18): Q3 and Q30 "who cares, continue" (the public fork,
  the full tree as the runnable fallback, one owner checkpoint before any push in P0-02); Q1 "I
  don't want running processes. When we close the toolbar widget, it stops, and is done." (one
  stop rule, A1: the `stop_policy` enum and its `last-ui`/`explicit` branches deleted everywhere,
  the popup's power button and confirm made explicit, the idle net kept at 300 s; Q2 and Q31
  answered with it; DL-70).

---

## 1. Where we are

### 1.1 Nothing is committed

| Layer | State | Evidence |
|---|---|---|
| `main` and `origin/main` (2026-09-03) | The legacy app: a qBittorrent-nox WebUI client with a 3-screen TUI. No `engine.rs`, `ipc.rs`, `shows.rs`, `rss.rs`, `search.rs` or `native_runtime*` in git. | [RC §2] |
| Branch `codex/shows-native-engine` | **Zero commits beyond `main`.** 19 modified files (+6363/-426) and about 3,575 untracked files, including `vendor/` (60 MB: patched Asupersync 0.5.0 at 57 MB, the rqbit 9.0.1 fork at 3.3 MB, the nova3 Python runtime at 64 KB) and `docs/validation/` (279 MB, 2,197 files, six dated directories). | `git status`, `du -sh docs/validation` |
| Installed `~/.local/bin/fbittorrent` | The native build from this working tree (21,555,840 bytes, 2026-10-14 05:55). The owner runs it daily. | [RC §2], `ls -la ~/.local/bin/fbittorrent` |
| Live data | `~/.local/share/fbittorrent/engine/torrents.json` ledger, RSS, Shows, search preferences; `shows.json` is mode 0644 while the others are 0600. | [RC §2] |
| Live config | `engine = "native"`, `network_interface` unset (VPN binding **off**), `save_path = ~/Downloads`, `autostart = true`, `seed_by_default = false`, `notify = true`. | `evidence/installed-config.json` |

A `git checkout`, `git clean` or a disk failure would turn the product back into the September
legacy app. P0-04 fixes this first. Until it lands, **no agent may run `git stash`, `checkout`,
`reset` or `clean` in this repository.**

### 1.2 Architecture today

```
 fbittorrent tui  (ONE process; needs a TTY: src/main.rs:452-455)
 ├─ FrankenTUI loop: TuiApp (src/tui/mod.rs:141-214, ~70 fields, 4 screens, 3 modals)
 │    every read/write: Cmd::task → qbt::Client → ipc::request over the socket, even to itself
 │    in-process: rss.rs, shows.rs, search.rs (Python child processes), imdb.rs
 │    ticks: Every 2000 ms → Msg::Poll; Every 300 ms → Msg::Hover (src/tui/mod.rs:1670-1674)
 ├─ ipc::Server: listener + ≤16 request threads (src/ipc.rs:174-263)
 │    $XDG_RUNTIME_DIR/fbittorrent/engine.sock, 4-byte length + JSON (src/ipc.rs:62-85)
 ├─ AppEngine: Asupersync runtime + librqbit Session + 1 s queue loop (src/engine.rs:395-499)
 │    NativeRuntime: 1 worker, 1-2 blocking threads (src/native_runtime.rs:44-50);
 │    SessionOptions asks for 2 engine workers and 96 peers (src/engine.rs:432-434)
 └─ lifecycle: statics QUIT_REQUESTED, RESTART, CLOSE polled by the TUI tick (src/tui/mod.rs:1699-1702)
 other processes: fbittorrent list/add/... (CLI) → same socket; Quickshell widget → spawns `status --json`
```

- **Transport:** one request per connection; a 4-byte big-endian length then JSON; 32 MiB cap
  (`src/ipc.rs:26,62-85`). Request `{method, path, args: [[k,v]], file: {name, bytes:[u8]}}`,
  response `{value: String|null, error: String|null}` with JSON double-encoded inside `value`
  (`src/ipc.rs:29-56`). No versioning; `app/version` returns `"native librqbit 9.0.1"`
  (`src/engine.rs:588`). Routes mimic the qBittorrent WebUI v2 [RC §7].
- **Security:** directory 0700, socket 0600 (`src/ipc.rs:143,187`); no peer-credential check; the
  HTTP method is ignored on engine routes [RC §7].
- **Who starts the app:** `connect()` → `backend::ensure_running` → `open_app()`, which launches
  the TUI in a terminal and polls up to 60 s for the socket (`src/main.rs:775-778`,
  `src/backend.rs:17-30,62-86`). `status` never starts it (`src/main.rs:407-432`).
- **Feature code location:** RSS and search run in the calling process in native mode
  (`src/qbt.rs:547-627`), Shows in `src/main.rs:1022+` and the TUI. That is why `fbittorrent search`
  works with the app closed and why search jobs die with the TUI [RC §7].
- **Agent surface:** `Code` 0..8 (`src/agent.rs:12-29`), `capabilities_json` (`src/agent.rs:134-210`),
  `triage` (`src/agent.rs:296-384`), the error envelope `{ok:false, code, code_name, error, hint}`
  (`src/agent.rs:113-123`), contract version `1.1.0` (`src/agent.rs:7`).

### 1.3 Live defects and where this plan fixes them

| # | Defect | Evidence | Fixed by |
|---|---|---|---|
| D1 | The widget's 60 s `reconcile` timer can reopen the app after the user quit it. | `plugin/oliver.torrents/Service.qml:121-126` → `src/main.rs:627-632` → `connect()` → `open_app()`; proven in `evidence/reconcile-autolaunch-probe.txt` | P0-07 |
| D2 | Closing the terminal window kills the process without running `Drop`, leaving a stale `engine.sock`. | Only SIGPIPE is handled by the app (`src/main.rs:288`); ftui's own signal thread force-exits with `128+sig` when the runtime does not acknowledge within `SIGNAL_SHUTDOWN_GRACE = 2 s` (`ftui-core-0.6.0/src/terminal_session.rs:107,147-167,1270-1295`); the runtime's loop polls the signal (`ftui-runtime-0.6.0/src/program.rs:5254-5441`) but clears it only after `shutdown_once` in `complete_lifecycle` (`:5484-5490`), which matches the observed 2.2 s (`evidence/app-lifecycle-observations.txt`); `Drop` at `src/engine.rs:517-546`, `src/ipc.rs:278-286` never runs | P0-08, then P1-03 |
| D3 | Every open re-hashes all data (7 s for the owner's 3.4 GiB). | `fastresume: false` (`src/engine.rs:437`); the fork refuses fastresume (`vendor/rqbit/librqbit/src/session.rs:567-568`, `torrent_state/initializing.rs:113-114`) | P1-16 (completed torrents), P6-06 (all) |
| D4 | One bad record blocks startup (each restore uses `?` and a 90 s metadata timeout). | `src/engine.rs:455-463`, `213-219` | P0-09 |
| D5 | "Resume" on a finished, stop-on-complete torrent reports "started" and does nothing. | `src/tui/mod.rs:2040`, `1325-1328`; `src/engine.rs:306-310` | P0-10 |
| D6 | Seeds are always 0; every peer is reported as a leecher. | `num_seeds: 0` (`src/engine.rs:1480`); `src/tui/mod.rs:2199`; `Model.js:79` | P0-11 (labels), P5-01 (real count) |
| D7 | The agent contract says the native upload cap is "1 B/s"; it is 32 KiB/s, and the engine rejects caps under 16 KiB/s. | `src/agent.rs:186,221` vs `src/qbt.rs:44`, `src/engine.rs:63-65` | P0-12 |
| D8 | Destructive confirms default to yes (Enter confirms "DELETE … AND ITS FILES?"); the widget removes on one click. | `src/tui/mod.rs:1260`, `1346-1348`; `Service.qml:62-65`; `Panel.qml:450-459` | P0-13 |
| D9 | Quitting stops every transfer without warning. | `src/tui/mod.rs:1178`, `1207-1212`; `Panel.qml:179`, `226` | P1-15 |
| D10 | `shows.json` and the ledger `engine/torrents.json` are world-readable (0644). | [RC §2]; both re-checked in round 4 | P0-14 |
| D11 | The documented multi-crate engine test commands no longer build (`sha1-crypto-hash` meets `sha1-ring`). | `vendor/rqbit/librqbit-core/Cargo.toml:30`, `vendor/rqbit/librqbit/Cargo.toml:33`; `evidence/cargo-test-engine-libs-combined-FAILED-BUILD.txt` | P0-05 |
| D12 | A `magnet:` click opens a terminal. | `src/backend.rs:62-86`; the desktop entry runs `add %u` (`src/setup.rs:112-114`) | P1-04 |
| D13 | The widget polls once per monitor. | `Service` instantiated inside each bar widget (`Panel.qml:119-122`) | P2-01 |
| D14 | README claims that are not true: YTS "browser-only", an UPLOADED column, dedupe by info hash. | [RC §5] | P0-12, P4-07 |
| D15 | Search shows duplicates and raw mojibake. | `src/tui/mod.rs:2979-2997`; `screens/07-search-sintel-16s.txt` | P3-01 |
| D16 (new) | The TUI's "dim" colour is `dark_foreground` (`#505050`, about 2.6:1 on black) even though the theme also ships `muted = #7a7a7a` (about 4.9:1); `theme.rs` loads `muted` and then overwrites it. | `src/tui/theme.rs:66-67`; `colors.toml` | P4-05 |
| D17 (new) | The tab bar spends eight cells on the theme name, which reads "omarchy" because `current/theme` is a directory, not a symlink. | `src/tui/theme.rs:74-77`; `screens/01-torrents-160x45.txt` line 1 | P4-07 |
| D18 (new) | The menu's "Stop" entry is gated on a qBittorrent unit and its "Open" entries launch the TUI in a terminal. | `~/.config/omarchy/extensions/omarchy-menu.jsonc:33-36` | P2-07 |

### 1.4 Feature surface

Of 36 qBittorrent features audited in native mode, 9 exist, 9 are partial and 18 are missing
[RC §8]. Present: add magnet/.torrent/URL, sequential and first/last, queueing, RSS feeds, search
plugins, encryption, VPN binding, the magnet handler, theming. Partial: file priorities
(include/skip only), pause/resume (no force-start), categories, tags, per-torrent limits (upload
only, and a change rebuilds the torrent), share limits, piece map (progress bar only), statistics
(served, never shown), port/UPnP. Missing: recheck, reannounce, rename, move, global limits,
alternative limits and scheduler, super seeding, peers view, trackers view, speed graphs, log,
RSS auto-download, torrent creator, IP filter, proxy, DHT/PeX/LSD toggles, remote backend,
completion notifications. §6.2 decides each one.

Beyond qBittorrent the app already has a Shows watchlist with an unread inbox, IMDb and Wikipedia
cards with posters, a provider browser, VPN wait-and-restore and an agent-ergonomic CLI. Neither
OmaTorrent project has these [OR §4].

### 1.5 UX today

Heuristic score 5.3/10 [RC §9]. The worst problems: state is shown only by colour below 140
columns, and the owner's real tiled window is about 90 columns wide (`screens/tui-real.png`);
the active monochrome theme makes red `#a4a4a4` and green `#b6b6b6` indistinguishable; dimmed
text is `#505050` on black (about 2.6:1); destructive prompts default to yes; quitting stops
everything without a word. The full list of 15 defects and their fixes is in §5.7.

The popup is built from the right components (`Panel`, `KeyboardPanel`, `PanelHero`,
`PanelActionButton`, `CursorSurface`, `PanelToolTip`: `Panel.qml:10,155,202,302,332`) and already
has a two-line row with a thin progress rule (`Panel.qml:332-460`). What makes it feel second-rate
is not the components but the data and the behaviour behind them: polled state up to 10 s stale,
"0s" seed counts, one-click removal, a `Qt.darker(fg, 1.55)` dim that differs from the shell's
own 1.4 (`Ui/PanelHero.qml:23`), a literal 400 ms animation that ignores the shell's
reduce-motion flag (`Panel.qml:410` vs `Commons/Style.qml:363-367`), and no window to open.

### 1.6 Tests today

| Suite | Result on 2026-10-18 | Notes |
|---|---|---|
| `cargo test --release` (app crate) | 90 passed | TUI interaction 21, shows 11, search 11, legacy client 11, engine 10, rss 8, imdb 4, misc 14 |
| `native.py` (isolated XDG, real tmux TUI) | 11/11 steps | The best end-to-end test; reused by P1-07. Steps print `ok: <label>` lines (`native.py:172-376`); there is no per-step log file yet |
| Engine libraries | 182 passed, 4 ignored, run per crate only | Combined commands fail to build (D11) |
| `run.sh`, `tui_smoke.sh`, `widget_keys.sh` | not run | Need legacy qBittorrent or the live shell |
| `vpn.py`, `resources.py` | not run | Namespace fixture; resource measurement with a `release-perf` build |

### 1.7 Code-health facts that shape the plan

- `src/tui/mod.rs` is 3,504 lines with one `TuiApp` of about 70 fields; footer hints, help text
  and key handlers are three hand-maintained copies of the key map (`2037-2104`, `2642-2721`,
  `1160-1646`); mouse hit-testing re-derives the layout (`796-843`) [RC §10].
- `src/engine.rs` (2,694 lines) mixes ledger persistence, queue policy, string-route dispatch and
  the parked server. Route names are matched in four places (`src/engine.rs:549-571`, `586-651`,
  `1233-1246`, `src/network_app.rs:78-90`). The qBittorrent state vocabulary exists three times
  (`src/engine.rs:1396-1440`, `src/tui/mod.rs:2933-2949`, `Model.js:32-57`).
- Lifecycle is process-global statics (`src/engine.rs:32`, `src/network_app.rs:9-11`).
- `qbt::Client` branches native/legacy in every method (`src/qbt.rs:278-366`, `547-629`).
- Positive: durable fsync'd writes everywhere, bounded subprocess capture with process-group kill
  (`src/search.rs:583-688`), a single-instance lock, no TODO debt in app code.
- The pinned FrankenTUI 0.6 already ships the widgets the TUI design needs: `table.rs`,
  `tabs.rs`, `sparkline.rs`, `virtualized.rs`, `log_viewer.rs`, `toast.rs`,
  `notification_queue.rs`, `badge.rs`, `scrollbar.rs`, `command_palette/` in
  `ftui-widgets-0.6.0/src/`, and `charts.rs`, `image.rs`, `canvas.rs` in `ftui-extras-0.6.0/src/`
  (round 0 cited ftui 0.7 for this [FO §3.3]; the pinned version suffices, so no upgrade is on the
  critical path).
- **Effort lesson.** October 11 to 14 went into download-CPU qualification (231 MB of receipts)
  while the user-visible gaps waited and nothing was committed [RC §10]. Download CPU is still
  about 3.2x qBittorrent's. This plan schedules no further CPU optimisation; it keeps a resource
  regression gate (P0-06) so changes cannot make it worse unnoticed.

---

## 2. Product definition

### 2.1 Who it is for

One person: the owner, on an Omarchy (Hyprland + Quickshell) desktop in Switzerland, downloading
for legal personal use. They stream what they download, so the first minutes of a file matter more
than total throughput. They work keyboard-first, use AI agents that drive the CLI, and sometimes
reach the machine over SSH. They do not seed by default and do not run a server. The product is
not aimed at the Omarchy marketplace (§7 explains why and what it would take).

### 2.2 Principles (each one settles later choices)

1. **One owner of state.** The daemon owns torrents, feeds, shows, search jobs and caches. UIs
   render; they never compute state. Why: two UIs drift unless the semantics live in one place
   [FO §7.3], and the Quickshell plugin must hold no state because every plugin save or shell
   restart reloads it [FO §2.7].
2. **Deliberate downloads.** Nothing downloads without an explicit user action. RSS refreshes and
   Shows checks never add torrents. Why: the owner's stated workflow is "find, inspect,
   deliberately download" [UX "One interaction model"].
3. **Streaming-first defaults are intrinsic.** Sequential plus first/last always on; seeding off
   by default. Why: owner decision of 2026-09-03. Round 1 adds: the UI must show when streaming
   can start (A6).
4. **Truthful state.** An unknown value is absent, never a fake zero or a sentinel; state is never
   shown by colour alone. Why: today's "0 seeds", `eta: 8640000` and colour-only state are
   defects D6, D9 and [RC §9] #1-#2; Sprout's "-1 B" free-space bug shows the same failure
   [OR §6 gotcha 6].
5. **On demand and visible.** Nothing at login. The daemon starts when a user or agent action
   needs it, shows in the bar while it runs (lit icon = running process), stops when the owner
   clicks the icon's power button (or runs `down`, or presses `Q`), and stops itself when idle (A1).
6. **API first.** Every feature lands as an API operation and topic with fixture tests before any
   UI uses it. Why: this is the only defence against the two UIs drifting [FO §7.3].
7. **Same letters everywhere.** A key means the same thing in the TUI, the dashboard and the popup
   unless the platform forces otherwise (Appendix B lists the exceptions; there are two).
8. **Omarchy-native look.** Shell components and theme tokens only; no literal colours; the shell's
   spacing, type and motion rules (§4.2). Why: the owner's ask; ucmz851's hard-coded palette is
   the counter-example [OR §5].
9. **The agent CLI is a first-class surface.** `--json`, the error envelope, exit codes,
   `capabilities` and `robot-docs` stay stable within contract 1.x; stdout is data, stderr is
   diagnostics; reads never start the daemon.
10. **Tests that explain themselves.** Every end-to-end case writes a JSONL log with what it
    expected and what it saw, and a failure prints the daemon's log next to it (§9.1).

### 2.3 Core workflows

Each workflow lists the surfaces involved and what "done" looks like. The work items in §9 trace
back to these.

**W1. Find, inspect, deliberately download, watch.**
1. Open the dashboard (`fbittorrent dashboard`, a keybinding, the menu, or `w` in the popup), or
   the TUI. Press `2` (Search) and type a query.
2. Results stream in from the enabled providers, deduplicated by info hash, with size, seeds,
   uploaded date, provider and IMDb rating. Resting on a result shows its poster card; an empty
   query shows the browse lists as a poster grid (P3-17).
3. Enter stages the result in the add sheet (dashboard) or downloads it (TUI, popup). The torrent
   starts sequentially with first/last pieces prioritised and appears in Torrents.
4. The row turns "Ready to watch" when the first minutes are on disk (A6); `o` opens the file in
   the default player; the notification's "Open" does the same.
Done when: a result can be downloaded with one key from either UI, the torrent shows progress
within 2 s, "Ready to watch" appears before completion on a video, and `o` opens the file.

**W2. Follow shows.**
1. On a search result, RSS item or torrent, `f` follows the show; on Shows, `n` follows by name or
   IMDb id; `=` (dashboard: the quality dropdown) sets the preferred quality per show (P3-18;
   `Q` stays quit everywhere, Appendix B).
2. When a UI opens and the last check is older than 30 minutes, the daemon checks EZTV. New
   releases appear in "All unread", newest first, and the Shows tab shows the unread count. With
   the daemon running in the background, a check that finds new releases notifies once (P6-10).
3. Enter downloads a release; `m` marks it read; `u` undoes the last inbox read.
Done when: the inbox survives restarts, a check never downloads anything, and read state and
transfer state are shown independently.

**W3. Glance at the bar.**
1. While fbittorrent runs, the bar shows an icon whose state (downloading, idle, ready to watch,
   waiting for VPN, error) is visible without opening anything; the tooltip gives rates and counts.
2. Clicking opens the popup: rates, VPN state, active downloads with progress and ETA, collapsed
   counts of the rest, and actions (add from clipboard, open dashboard, search, turtle, quit).
Done when: the popup reflects a change within one push interval, and the shell spawns no
processes for status.

**W4. Magnet from the browser.**
1. The browser hands `magnet:` or a `.torrent` file to `fbittorrent add %u`.
2. The CLI starts the daemon if needed (through systemd), adds the torrent and posts a
   notification with "Undo" and "Open window" actions (A5).
Done when: no terminal window opens, and the torrent is visible in every UI within a second of
being added.

**W5. Agent and CLI use.**
1. An agent runs `fbittorrent robot-docs` once, `triage --json` for state, `search … --json`,
   `add … --wait --json`, `show <hash> --json | jq .open_target`, `events --follow --json` to
   react to completions without polling, `wait --until playable` to start a player early.
2. Read commands never start the daemon; when it is down they read the durable ledger and say
   `running: false`. Write commands start it.
Done when: the capabilities contract (1.2.0) documents every change and the agent regression
tests pass.

**W6. Over SSH.** `ssh host fbittorrent tui` attaches to the running daemon (or starts it) and
offers the same features as locally; quitting the TUI leaves downloads running under policy A1.

**W7. Downloading bound to the VPN.** With `network_interface = "proton"`, torrent traffic uses
only `proton0`. When the tunnel drops, transfers stop and every UI shows "Waiting for ProtonVPN";
when it returns, saved intent resumes. Search, RSS and IMDb use the normal route as today
(`README.md:79-86`).

**W8. Clean up.** After watching, the owner removes finished torrents (keeping or deleting
files) one at a time or as a filtered set, always through an explicit choice between "Remove" and
"Remove and delete files", with a 10 s undo for the keep-files case (P6-08).

### 2.4 Surfaces and their jobs

| Surface | Job | Starts the daemon? |
|---|---|---|
| Bar icon | Ambient state: off, idle, downloading, ready to watch, waiting for VPN, error. | No |
| Bar popup | Glance and quick act: progress, pause, open, add from clipboard, remove (keep files), turtle, quit. | No (hidden while the daemon is off by default) |
| Dashboard window | The full app: Torrents with details, Search, Shows, RSS, Settings, Log. | Yes, when opened |
| TUI | The full app in a terminal, over SSH, and the fallback when the shell is broken. | Yes |
| CLI | Agents and scripts; `--json` everywhere; `events --follow`. | Writes yes, reads no |
| `magnet:` / `.torrent` handler | Add from the browser. | Yes |
| Notifications | Added (with Undo), completed, ready to watch, failed, VPN waiting, new episodes, clipboard magnet (opt-in). | n/a |
| Omarchy menu (`torrents.*`) | Open dashboard, search, add from clipboard, quit. | Opening yes |

### 2.5 Fixed owner constraints and how the plan honours them

| Constraint (owner, 2026-09-03 unless noted) | How |
|---|---|
| Binary name `fbittorrent` | One binary; `serve` is a subcommand. |
| Personal, legal use in Switzerland | §7: no marketplace goal; providers stay user-installed scripts. |
| Sequential + first/last always on | Unchanged engine; API has no toggle (§3.5); A6 builds on it. |
| Seeding off by default, 32 KiB/s while downloading, stop when complete, per-torrent `seed` | Unchanged; the API exposes `seed: bool` as one switch instead of the cap/ratio/tag trio (`src/qbt.rs:504-520`). |
| Downloads to `~/Downloads` | Unchanged default. |
| Nothing at login, backend on demand | Static systemd unit; §3.2.3 lists who may start it. |
| Bar widget uses native `Panel` chrome, never a tray icon | §4.5 |
| Default terminal Ghostty | The TUI launcher is unchanged (`omarchy-launch-or-focus-tui`, `/usr/share/omarchy/bin/omarchy-launch-or-focus-tui`). |
| Theme from the active Omarchy theme, live | QML binds to `qs.Commons`; the TUI keeps its reload (`src/tui/mod.rs:779-794`). |
| RSS is add/remove feeds and see items | §6.2 keeps auto-download out. |
| IMDb cards without an API key | §3.7, unchanged sources (IMDb ratings dataset, suggestion endpoint, Wikipedia summaries: `src/imdb.rs:1-4,13`). |
| Keys as documented in README | Appendix B keeps them; changes are additions or aliases. |
| Agent surface stays | P1-11 (contract 1.2.0, additive), Appendix F. |
| 2026-10-18: likes OmaTorrent's format and capabilities | Daemon + Quickshell popup + dashboard (§3, §4) and the parity work (§6). "Format" means a native Omarchy surface fed by a daemon rather than a terminal app [OR §2.1]; "capabilities" means the turtle toggle, completion notifications, the magnet confirm and a details view [OR §4], which §6.1 makes T1 and §4.8 adapts (staged add instead of a confirm popup). |

### 2.6 Non-goals

| Non-goal | Why, for this owner | Revisit if |
|---|---|---|
| Torrent creator | The owner consumes, does not publish. The library code exists but is unexposed (`vendor/rqbit/librqbit/src/create_torrent_file.rs`). | They start sharing their own files. |
| IP filter / blocklists | VPN binding and encryption cover the privacy need; blocklists need upkeep. | A concrete abuse case appears. |
| Super seeding | Seeding is off by default. | Never, realistically. |
| Remote qBittorrent backend (Sprout's main feature) | The owner left qBittorrent on purpose. | n/a |
| Network access to fbittorrent (WebUI, TCP API) | A localhost HTTP surface is the CSRF/DNS-rebinding risk the project already avoided (`README.md:226-235`). SSH plus the TUI covers remote use. | A phone UI becomes a real need; then design auth first. |
| SOCKS/HTTP proxy | VPN interface binding is the chosen mechanism; the fork has no SOCKS ([EE] "Upstream service APIs"). | The VPN provider changes to a proxy model. |
| RSS rule engine | Owner decision: RSS is feeds and items. Shows covers episodic follow. | The owner asks for Shows auto-grab (§10.3 Q5). |
| Alternative-speed scheduler | No background schedule fits an on-demand daemon; manual turtle (`z`) covers the need. | The daemon becomes long-running by policy (c). |
| Categories with save paths | One owner, one download folder; an `origin` field (search, RSS, shows, manual) gives the useful grouping (P6-08). | Multiple libraries appear. |
| High/max file priorities | Sequential scheduling makes priority mostly moot; the engine refuses them on purpose (`src/engine.rs:657`). | n/a |
| Tracker scrape for swarm-wide seed counts | `seeds` means connected seeds (DL-19); a scrape adds tracker traffic for a number that does not change what the user does. | The owner asks why a torrent with 0 connected seeds is listed with 800 on the site. |
| Web seeds (HTTP sources) | No source the owner uses publishes them. | A provider does. |
| Automatic torrent management (qBittorrent's category-path rules) | One folder. | n/a |
| uTP, BitTorrent v2 | Large engine work with no felt benefit so far [RC §12]. | Measured swarms need them. |
| Tray icon, autostart at login | Owner constraints. | n/a |
| A Rust GUI toolkit | A third UI technology without the Omarchy look [FO §3.4]. | Quickshell becomes unusable (§10.1 R2 fallback). |
| Omarchy marketplace listing | §7. | The owner asks; §7.4 says what it would take. |
| File rename inside a torrent | Breaks seeding and re-checks; a file manager does it after completion. | n/a |

### 2.7 Better than qBittorrent, for this user

qBittorrent is the yardstick the owner left. Parity (§6) matters, but the plan is only worth
doing if the result is better for a streaming-first, keyboard-first, agent-driven single user.
These are the capabilities that qBittorrent (and both OmaTorrent projects, [OR §4]) do not have,
each with the item that delivers it and the test that proves it:

| Capability | What the user sees | Item | Proof |
|---|---|---|---|
| Ready to watch | A row says "Ready to watch · 4 min buffered" as soon as the contiguous prefix of the largest video file crosses the threshold, and "Playable · may stall (finishes in 18 min)" while the current rate cannot keep ahead of playback; a notification offers "Open" (and opens on a body click); `o` opens it; `fbittorrent wait --until playable` and `--until stall-free` return for agents | P5-05, P5-06 | `daemon.py::playable_*` with a local seed throttled to 1 MiB/s on a 64 MiB fixture: the event fires before completion and `open_target` plays |
| Deliberate add | Dashboard add sheet with files, seed switch, destination and free space; `Esc` discards without touching files | P3-06, P3-11, P6-11 | `daemon.py::staged_discard_keeps_files` |
| Instant magnet, no terminal | Click, notification with Undo | P1-04, P1-13 | `daemon.py::magnet_handler_no_launcher` with a `PATH` shim |
| Show inbox with quality | Followed shows, unread inbox, per-show quality, new-episode notifications | P3-13, P3-18, P6-10 | EZTV fixture cases |
| Poster-rich browsing | Search results and browse lists with IMDb ratings and posters; a poster grid for the lists | P3-12, P3-17 | qs smoke with the poster fixture |
| Truthful bar | Icon state at a glance; no "0 seeds"; pushed updates | P2-04, P5-01 | screenshot checklist; `peers.ndjson` |
| Agent ergonomics | `triage`, `events --follow`, `api`, `wait --until`, typed errors with hints, exit codes 0..8 unchanged | P1-11, P1-18 | `agent_contract.py` (the successor of `run.sh`) |
| Nothing runs when nothing happens | Idle exit with a visible countdown; `status` never starts anything | P1-12 | `daemon.py::idle_exit_*` |
| Clipboard magnet (opt-in) | "Add 'Sintel'?" strip in the popup and a notification when a magnet lands on the clipboard | P6-13 | `daemon.py::clipboard_watch_*` with a fake `wl-paste` |
| Undo and bulk cleanup | Remove keeps a 10 s undo; filtered bulk remove shows count and size | P6-08 | `daemon.py::undo_restore` |
| VPN-bound and honest about it | "Waiting for ProtonVPN" everywhere; never a fallback route | existing + P1-03 | `vpn.py` namespace fixture |

---
## 3. Architecture

### 3.1 Target topology

```
 user session (Hyprland + omarchy-shell)
 ┌──────────────────────────────────────────────────────────────────────────────┐
 │ omarchy-shell (one Quickshell process)                                       │
 │   plugin oliver.torrents                                                     │
 │     Service.qml    kind service     one api.sock connection, Store, actions  │
 │     BarWidget.qml  kind bar-widget  icon + KeyboardPanel popup, per monitor; │
 │                                     reads Service, owns no timers/processes  │
 │     Dashboard.qml  kind panel       FloatingWindow titled "fbittorrent"      │
 │ Ghostty: fbittorrent tui            pure API client (after P4-03)            │
 │ CLI, magnet handler, agents         short-lived API clients; `events` streams │
 └───────────────┬──────────────────────────────────────────────────────────────┘
                 │ NDJSON over $XDG_RUNTIME_DIR/fbittorrent/api.sock
                 │ (dir 0700, socket 0600, SO_PEERCRED uid check)
                 │ legacy 4-byte framing on engine.sock until v0.4.0
                 ▼
 systemd --user: fbittorrent.service (static: no [Install], never enabled; Type=notify)
 ┌──────────────────────────────────────────────────────────────────────────────┐
 │ fbittorrent serve                                                            │
 │   DaemonLock (engine.lock flock)      Lifecycle (stop and restart reasons)   │
 │   API server: hello, ops, subs        Event bus: snapshot, deltas, revisions │
 │   Engine slot: Starting | Running | Parked | Stopping (outlives restarts)   │
 │   Supervisor: VPN watch, settings restarts, idle net, upgrade check,         │
 │               watch-readiness, clipboard watch (opt-in)                      │
 │   Feature services: search sessions, RSS, Shows, IMDb cache, browse lists    │
 │   Notifier (notify-send with actions), log ring, sleep inhibitor (P6-09)     │
 └──────────────────────────────────────────────────────────────────────────────┘
   state: ~/.local/share/fbittorrent/{engine/,rss.json,shows.json,search.json}
          ~/.cache/fbittorrent/imdb   ~/.local/state/fbittorrent/{stats.json,notices.json}
```

### 3.2 The daemon

#### 3.2.1 Command

`fbittorrent serve` runs the daemon in the foreground and logs to stderr (journald captures it
under systemd). Flags: `--idle-exit-secs N` and `--no-notify` (both for tests). The clients'
protocol-mismatch banners are exercised from the fixture `contracts/ipc/v1/hello.ndjson` (an
`s2c` `hello` with `proto: 2`), which `Store.js` and `api::Client` consume (P1-05, P2-02), so the
daemon needs no flag to lie about its version. It appears in
`--help` as "Run the torrent engine daemon (other commands start it on demand)". All paths derive
from the XDG variables (`src/config.rs:140-147`), so tests isolate a daemon by pointing `XDG_*` at
a temporary directory, exactly as `native.py` does today (`native.py:44-50`).

A second `fbittorrent serve` while one holds the lock exits 1 with the typed error
`already_running` and the hint `fbittorrent status --json`; systemd never starts two instances of
one unit, so this only happens when a human runs `serve` by hand.

#### 3.2.2 The systemd user unit

`fbittorrent setup` writes `~/.config/systemd/user/fbittorrent.service` with the absolute path of
the running binary and runs `systemctl --user daemon-reload`:

```ini
[Unit]
Description=fbittorrent torrent engine (started on demand)
Documentation=https://github.com/odcpw/fbittorrent
# On-demand units start often by design (idle exit, exit 75 upgrades, tests); the default
# 5 starts per 10 s limit (`systemctl --user show`: DefaultStartLimitBurst=5, 10s) would otherwise
# wedge the unit until `systemctl --user reset-failed`.
StartLimitIntervalSec=60
StartLimitBurst=20

[Service]
Type=notify
NotifyAccess=main
ExecStart=/home/<user>/.local/bin/fbittorrent serve
KillSignal=SIGTERM
# SIGTERM to the daemon only; it drains providers (`src/search.rs:945-961`), the inhibitor and
# notifier children itself. Anything still alive at TimeoutStopSec is SIGKILLed by systemd.
KillMode=mixed
TimeoutStopSec=20
Restart=on-failure
RestartSec=2
# Exit 75 means "binary replaced while idle; start the new one" (§3.2.12).
RestartForceExitStatus=75
SuccessExitStatus=75
NoNewPrivileges=yes
# No [Install] section on purpose: the unit is static; `systemctl --user enable` has nothing to
# link (it only warns), so nothing can start it at login.
```

Why `Type=notify`: `systemctl --user start` then returns exactly when the daemon accepts
connections, which replaces today's 60 s socket polling loop (`src/backend.rs:17-30`). Readiness is
sent with a datagram to `$NOTIFY_SOCKET` (`READY=1`, later `STATUS=2 downloading · ↓ 4.2 MB/s`,
`STOPPING=1`); about 30 lines of std-only code, no crate. Why `TimeoutStopSec=20`: the existing
drain is bounded at 4 s in total, shared by the session stop and the runtime shutdown
(`shutdown_timeout(bound - elapsed)`, `src/engine.rs:517-546`); the rest is headroom for the
`bye` frames and the last ledger write.
Further hardening (`ProtectSystem`, `PrivateTmp`) is left out because user units with those
options need user namespaces and `ProtectHome` would hide `~/Downloads`; P1-04 may add options that
pass its tests. The user manager on this machine exports `WAYLAND_DISPLAY` and
`DBUS_SESSION_BUS_ADDRESS` (`systemctl --user show-environment`, checked 2026-10-18), so
`notify-send` and `xdg-open` work from the unit.

#### 3.2.3 Who may start the daemon

The rule: **writes start it, reads never do.**

| Caller | Starts the daemon? | When it is down |
|---|---|---|
| `add`, `add-clipboard`, `start`, `seed`, `noseed`, `wait`, `delete --files`, `recheck`, `limits`, `rss grab`, `shows grab`, `up`, `tui`, `dashboard`, the magnet handler | yes | n/a |
| `stop`, `delete` (keep files) | no | Applied directly to the durable ledger, reusing the parked code path (`src/engine.rs:1233-1355`). Order: `hello` fails with `ECONNREFUSED`/`ENOENT` → acquire `engine.lock` (`LOCK_EX\|LOCK_NB`, retried for 2 s; if it stays held the daemon is starting or draining, so retry `hello` instead) → read the ledger → write it (temp, fsync, rename, dir fsync) → release. The daemon's own 5 s lock retry at start covers the overlap. Starting an engine only to pause something is pointless. |
| `status`, `list`, `show`, `triage`, `config`, `capabilities`, `robot-docs` | no | Read the ledger; report `running: false`; progress is absent (unknown). |
| `log`, `events` | no | Exit 4 with the hint `fbittorrent up`, with or without `--follow`: the log ring and the event ring exist only inside the daemon (journald keeps the long log history: the hint also prints `journalctl --user -u fbittorrent`). |
| `search`, `imdb`, `rss list/items/refresh/add/remove`, `shows list/episodes/check/follow/unfollow/seen` | no | Run in-process as today (§3.7). |
| `settings` | only if a running daemon must apply it | Saved to config; applied at next start. |
| `api <op>` | only when `<op>` is mutating (Appendix F) | Reads against a stopped daemon exit 4 with the hint `fbittorrent up`. |
| Bar service connection | never | Shows "off" (or stays hidden, §4.5). |
| Opening the dashboard window | yes | It is an interactive UI. |
| Any command with `--no-start` or `FBITTORRENT_NO_START=1` | never | A command that would have started the daemon exits 4 with a hint; reads still fall back to the ledger with `running: false` and exit 0. |
| Any command when the config has `autostart = false` (the existing key, `src/config.rs:39`, README:164-169) | never, except `up`, `tui`, `dashboard` and `serve`, which are explicit | A command that would have started the daemon exits 4 with the hint `fbittorrent up`; reads fall back to the ledger as above; this is the persistent form of `--no-start` and keeps today's documented meaning (`src/agent.rs:182`). |

#### 3.2.4 Start mechanics

`ensure_daemon()` in `src/api/client.rs`:

1. Connect to `api.sock` and send `hello` (200 ms connect timeout). Success ends here.
2. If `--no-start` or `FBITTORRENT_NO_START=1`: exit 4, "fbittorrent is not running".
3. If the unit file exists and `systemctl --user show-environment` succeeds: run
   `systemctl --user --no-ask-password start fbittorrent.service`, bounded to 30 s by the caller.
   It returns when the daemon has sent `READY=1`, or at once when the unit is already active.
   Retry `hello`; on `ECONNREFUSED` or `state: "stopping"` the unit was exiting when `start`
   looked at it, so wait for `systemctl --user is-active` to leave `deactivating` (poll 100 ms,
   at most 10 s) and run `start` once more. Two failed rounds are a failure.
4. Otherwise (no user manager, §3.2.14): exit 4 with the hint `fbittorrent serve` ("run the daemon
   in a terminal or a tmux window; this session has no systemd user manager").
5. On failure: exit 4 with the hint `journalctl --user -u fbittorrent -n 50`.
6. **Mid-request loss.** If the connection ends (EOF, `bye {reason: idle|restart|upgrade}`) after a
   request was sent and before its response: for a read, or for a mutation whose op is marked
   idempotent in A.1, re-run steps 1-4 once and resend with the **same `ref`**; for
   `remove`, `recheck`, `move` exit 8 (`timeout`) with the hint `fbittorrent list --json` and the
   message "the daemon stopped while this request was in flight; check the list". Clients never
   resend after `bye {reason: slow_client|hello_required|invalid_message}`.

`READY=1` is sent once the daemon's socket listens (`engine.sock` from P1-03; `api.sock` as well
from P1-06) and the engine slot exists in `Starting`, not after
hash checks finish. The slot becomes `Running` as soon as the librqbit session exists; saved
records are restored by background tasks (P0-09 already makes each restore independent) and show
as `checking` or `metadata` meanwhile, because librqbit admits new torrents while others
initialise (`concurrent_init_limit` only serialises the checks, `src/engine.rs:432`). Operations
that need the engine therefore wait only for the session (seconds, not minutes); `torrents.add`
keeps a 60 s bound and fails with retryable `engine_unavailable` past it. Why: under A1 the
daemon starts after every idle period, and today's `engine::start` awaits every restore in series
(`src/engine.rs:453-466`), which would put the whole library's check time between a magnet click
and "Added".

#### 3.2.5 Single instance, sockets and the runtime directory

- Runtime directory `$XDG_RUNTIME_DIR/fbittorrent`, falling back to `<data_dir>/runtime`
  (`src/config.rs:144-147`). The daemon validates it: absolute path, owned by the effective uid,
  mode 0700 (set when we created it; refuse to start if another uid owns it), not a symlink. Why:
  Sprout refuses ambiguous runtime directories for the same reason [OR §2.1].
- `engine.lock`: the existing `flock(LOCK_EX|LOCK_NB)` (`src/ipc.rs:136-164`), now held by the
  daemon for its whole life instead of by `AppEngine` or `ParkedEngine`. At start the daemon retries
  the lock for up to 5 s, because an offline `stop` or `delete` from the CLI (§3.2.3) may hold it
  for a moment.
- `api.sock` (new) and `engine.sock` (legacy, until v0.4.0) are unlinked and bound only while the
  lock is held. That makes stale-socket removal safe: whoever holds the lock is the only possible
  owner (the lock file itself is never unlinked, `src/ipc.rs:151`; the socket is unlinked under it,
  `src/ipc.rs:181-185`). A client that gets `ECONNREFUSED` from a
  socket file therefore knows the daemon is gone, not busy, and treats it as "stopped".
- Every accepted connection is checked with `SO_PEERCRED`; a different uid is logged and closed.
  Today there is no such check [RC §7].
- `instance` (a random 128-bit id in `hello`) changes on every daemon start, so a client can tell
  "the same daemon, my subscription is intact" from "a new daemon, resubscribe".

#### 3.2.6 Signals and panics

- In `serve` only, `SIGPIPE` is set to `SIG_IGN`. `main` resets it to `SIG_DFL` for CLI pipes
  (`src/main.rs:288`); a daemon that keeps that default dies the first time it writes to a client
  that went away.
- The main thread blocks `SIGTERM`, `SIGINT` and `SIGHUP` with `pthread_sigmask` before starting any
  thread, so every thread inherits the mask. One thread loops on `sigwait` and calls
  `lifecycle.request_stop(StopReason::Signal(n))`. Why: this is the race-free way to handle
  signals in a multi-threaded process without an async runtime, and it puts SIGTERM from systemd on
  the same drain path as `fbittorrent down`. The daemon never links ftui, so ftui's own signal
  thread (`ftui-core-0.6.0/src/terminal_session.rs:1270-1295`) exists only in the TUI client.
- A panic hook logs the panic to stderr and the log ring and requests a stop with exit code 70;
  `Restart=on-failure` then restarts the daemon. Worker panics are already treated as test
  failures by `all.sh` (its `rg` scan, `all.sh:12-16`); this makes them visible in production too.

#### 3.2.7 Daemon lifetime: the state machine

The daemon has one lifetime state and one engine slot. Both are visible to clients through
`hello.state`, `summary.daemon` and the `engine.changed` event, so every surface can explain what
is happening (§4.11, §5.5.3).

```
                 systemctl start / serve (terminal)
   ┌────────┐ ─────────────────────────────────────► ┌──────────┐ READY=1 sent when api.sock listens
   │ Absent │                                         │ Starting │ slot: Starting (ledger read; no session yet)
   └────────┘ ◄──── exit 0 / 70 / 75; sockets gone ── └────┬─────┘
        ▲                                                  │ engine::start ok     │ binding configured, device missing
        │                                   ┌──────────────┘                      └──────────────┐
        │                                   ▼                                                    ▼
        │                              ┌─────────┐  device lost: drain, park, notify once   ┌────────┐
        │                              │ Running │ ─────────────────────────────────────► │ Parked │
        │                              │         │ ◄───────────────────────────────────── │        │
        │                              └────┬────┘  device back: start engine, restore     └───┬────┘
        │   restart-class setting, `restart`│ (slot cycles Running→Starting→Running; connections stay)
        │                                   │
        │   stop requested (idle timer, `down`, `Q`, SIGTERM, panic, upgrade)                   │
        │                                   ▼                                                    │
        │                              ┌──────────┐ bye{reason} to every client; STOPPING=1       │
        └───────────────────────────── │ Draining │ ◄────────────────────────────────────────────┘
                                       └──────────┘ session.stop ≤4 s, runtime ≤4 s, unlink sockets, release lock
```

| Transition | Trigger | Observable |
|---|---|---|
| Absent → Starting | `systemctl --user start`, `fbittorrent serve` in a terminal (§3.2.14), or `Restart=on-failure` after a crash | unit active; `hello` answers with `state: "starting"` |
| Starting → Running | the librqbit session exists (DL-49, §3.2.4); saved records are restored by background tasks afterwards and show as `checking` or `metadata` meanwhile | `engine.changed {epoch: 1, state: "running"}`; fresh snapshots |
| Starting → Parked | binding configured and `network_app::state(cfg).ready == false` (`src/network_app.rs:22-36`) | `state: "parked"`; `network` topic says why; one notification |
| Running ↔ Parked | device loss or return, checked once per second by the supervisor | `engine.changed` with a new `epoch`; rows lose or regain `progress`, `dl_bps`, `peers` (§3.4.9) |
| Running → Starting → Running | restart-class setting (`peer_encryption`, `network_interface`, `peer_port`), `app.restart {scope: "engine"}` | `engine.changed` twice; connections survive (§3.2.8) |
| any → Draining | idle countdown reaches 0 (§3.2.10); `app.shutdown`; SIGTERM/SIGINT/SIGHUP; panic (exit 70); binary replaced while idle (exit 75) | **Quiesce first:** the listener stops accepting; every request already read is answered (or answered `engine_unavailable {state: "stopping"}`, retryable, if it needs the engine); then `bye {reason}` is the last frame on every connection and the socket is closed; `STOPPING=1`. A connection that completes `hello` during Draining gets `state: "stopping"` and `engine_unavailable` on every op. |
| Draining → Absent | drain done or `TimeoutStopSec` | sockets unlinked, lock released, unit inactive (or restarting) |

Crash recovery and stale state: if the process dies without draining (kill -9, OOM), the
`flock` is released by the kernel and the socket files remain. The next start unlinks them under
the lock (§3.2.5); clients that were connected see EOF and reconnect with backoff (§3.2.11); a
`status` call sees `ECONNREFUSED` and reports `running: false`. The ledger is fsync'd on every
change ([EE "Ownership and durable state"]), so no intent is lost; data is re-checked on the next
start (until P6-06). Nothing is ever "half-removed": `torrents/delete` saves the record removal
before touching the payload (`src/engine.rs` delete path; [EE]).

#### 3.2.8 Supervisor loop and engine slot

```
supervisor (one thread):
  every 1 s, or immediately when the lifecycle is signalled:
    stop requested                          → leave the loop
    restart-class setting changed            → restart_engine(reason = settings)
    binding configured and network changed   → restart_engine(reason = network)   (park or unpark)
    watch-readiness tick (P5-05)             → for each live video file: contiguous prefix → playable?
  every 5 s:  evaluate the busy table (§3.2.10); start, cancel or fire the idle countdown
  every 30 s: check whether the binary was replaced (§3.2.12)
shutdown:
  bus broadcasts bye{reason}; slot := Stopping
  drop the engine (existing drain: session ≤ 4 s, then runtime shutdown, src/engine.rs:517-546)
  stop the API server and the legacy adapter; unlink sockets; release the lock
  STOPPING=1; exit with the reason's code (0, 70 or 75)
```

The engine slot is an `Arc<RwLock<Slot>>` with `Slot::{Starting, Running(AppEngine),
Parked(ParkedLedger), Stopping}` and an `engine_epoch` counter. The API server and the event bus
belong to the daemon, not to the engine, so client connections survive engine restarts: on a
restart the bus sends `engine.changed {epoch, state}` followed by a fresh snapshot on each
torrent-like subscription. Why: today the parked mode builds its own socket server
(`src/network_app.rs:92-106`), so a restart drops every client; with a bar connection that would
mean a reconnect storm on every VPN flap.

The network check runs once per second here, only when binding is configured. Today the TUI calls
`network_app::state()` on every render (`src/tui/mod.rs:2148`), which queries the kernel [RC §10].

#### 3.2.9 VPN binding behaviour while headless

- Semantics are unchanged: engine sockets bind to the exact device, there is never a fallback to
  another interface, and `proton` means `proton0` (`src/network_app.rs:22-36`).
- At start, if binding is configured and the device is not ready, the slot starts `Parked`: the
  ledger is served for reads, plus start, stop and remove-keeping-files, as today
  (`src/engine.rs:1233-1355`). `torrents.add` fails with `engine_unavailable` and a hint to connect
  the VPN. (Deferred adds while parked are a T3 idea, P7-09.)
- Up to down: drain the engine, park, push `network` state and `engine.changed`, notify once
  ("Waiting for ProtonVPN"). Down to up: start the engine, restore saved intent, push both,
  notify once ("VPN back; 2 downloads resumed"). The one-notification-per-transition rule stops a
  flapping tunnel from spamming; the bar icon carries the state continuously (§4.5).
- With no UI open the daemon still parks and unparks: that is the point of headless binding, a
  download started before the tunnel dropped resumes by itself when it returns. The busy table
  counts a parked engine with waiting downloads as busy (§3.2.10), so the daemon does not exit
  while it waits.
- Search, RSS and IMDb traffic from the daemon uses normal routing, and provider download helpers
  stay refused while binding is on (`src/qbt.rs:413-423`), exactly as documented today
  (`README.md:79-86`).
- Messages name the configured interface instead of hard-coding "ProtonVPN" for every interface
  (`src/engine.rs:831`, `1248`) [RC §3 #19].
- The owner's live config has binding **off** (`evidence/installed-config.json`); R10 keeps the
  manual check before it is turned on again.

#### 3.2.10 Stop rule and idle exit (owner decision A1)

The owner's rule ("I don't want running processes. When we close the toolbar widget, it stops, and
is done."): the daemon runs while the bar icon is lit and stops when the owner stops it, from the
bar icon's power button (§4.5), from `fbittorrent down` or from `Q` in the TUI, each sending
`app.shutdown {reason: "user"}`; the popup and the TUI confirm first when transfers are active.
Nothing else stops it: closing the TUI or the dashboard window, dismissing the popup and losing the
bar connection never stop a transfer. The idle net below exits the daemon by itself when nothing is
left to do, so a forgotten daemon does not run for days. There is no `stop_policy` setting (the
former `last-ui` and `explicit` policies are gone, DL-70); the one knob is `idle_exit_secs`.

The daemon is "busy" while any of these holds:

| Busy reason (`summary.daemon.busy_reasons` value) | Busy? | Notes |
|---|---|---|
| `downloading`: a torrent with intent "run" that is incomplete (downloading, stalled, metadata, checking, queued) | busy | Staged torrents in the add sheet do not count. |
| `seeding`: a torrent with `seed = true` that is running | busy | An explicit user choice. |
| `parked_waiting`: parked (waiting for the VPN) with at least one download waiting | busy | Exiting would silently drop the intent; the bar shows the waiting state instead. |
| `interactive`: a TUI, or a visible dashboard window | busy | The bar icon never counts (§4.4 rule 8). |
| `glance`: the bar popup is open (`app.client {glance: true}`) | not busy; the countdown **pauses** while a glance is open and resumes where it was when it closes | A popup that announces "stops in 4 min" must not stop while it is being read; a glance never starts the daemon and never counts as interactive, so a forgotten open popup holds the daemon only as long as the popup itself stays open (the shell closes it on focus loss). |
| `search`: a search session | busy | Sessions end 30 s after their client leaves. |
| `mutation`: a pending long mutation (metadata resolution, payload deletion, recheck, move) | busy | |
| `clipboard_watch`: the opt-in watcher is on (P6-13) | **not busy** | The watcher restarts with the daemon; it never keeps it alive. |

- The idle net: when nothing is busy, a countdown of `idle_exit_secs` (default 300; minimum 30
  unless 0) starts; anything busy cancels it. `summary.daemon.idle_exit_in_s` exposes the remaining
  time so the popup can say "stops in 4 min". At zero the daemon sends `bye {reason: "idle"}` and
  exits 0. `idle_exit_secs = 0` disables the net: the daemon then stops only when the owner stops it.
- The owner's stop: `app.shutdown {reason: "user"}` from the power button, `down` or `Q` drains
  (§3.2.7 "shutdown") whatever the busy table says; downloads pause and resume when the daemon next
  starts, because their intent stays "run" in the ledger. The confirm when transfers are active
  belongs to the client (§4.5, §5.2), never to the daemon.

Why 300 s: it covers "close the popup, click another magnet a minute later" without a cold start
and its hash checks, and nothing lingers for long after the work is done. It is a setting.

#### 3.2.11 Crash handling

- `Restart=on-failure` with `RestartSec=2`; the unit's start limit (20 starts in 60 s, §3.2.2) stops a
  crash loop.
- The ledger is fsync'd on every change [EE "Ownership and durable state"], so a crash loses no
  intent.
- Clients see the socket close, show "reconnecting", and retry after 0.5, 1, 2, 4 and 8 s (then
  every 10 s). A successful reconnect brings a new `instance` id and a fresh snapshot. Why these
  numbers: a systemd restart after a crash takes `RestartSec=2` plus the time to `READY=1`, so
  the 2 s and 4 s retries catch it within one or two attempts without hammering a daemon that is
  crash-looping (the unit allows 20 starts per minute, §3.2.2); the 10 s cap is the old widget's
  idle poll (`manifest.json:19`), so the bar is never staler than it is today; and the same
  ladder is §4.4 rule 4, so QML and Rust clients behave alike in tests.
- A mutation in flight at crash time is ambiguous. UIs say "result unknown; check the list" and
  never retry a remove automatically [OR §8 item 3]. The `ref` ring (§3.4.8) is per instance and
  dies with the process on purpose: replaying a pre-crash ref against a new instance is exactly
  the double-apply the ring exists to prevent, so the new daemon applies it as a fresh request
  only for the idempotent operations of A.1 and refuses every other mutation with
  `invalid_state` ("daemon restarted since this request; check the list").

#### 3.2.12 Upgrades and version mismatch

- A `build.rs` exports `FBT_BUILD_ID`: the short git hash plus `-dirty`, or `unknown`. `hello`
  returns `server.version` (`CARGO_PKG_VERSION`) and `server.build`.
- Same version and build: silent. A different version or build: the CLI adds an entry to
  `warnings[]` in JSON and prints one stderr line in human mode ("daemon 0.2.0+564fa18 is running;
  this binary is 0.2.1+9c1d2e0; run `fbittorrent restart` to switch"); the TUI and the plugin show
  a dismissible banner. An unsupported protocol major is a hard error, `proto_unsupported`, with
  the same hint.
- Binary replacement: at start the daemon records `(dev, inode, mtime)` of its `current_exe()`
  path and checks it every 30 s. If it changed and the daemon is not busy, it exits with code 75;
  the unit restarts it at once with the new binary (`RestartForceExitStatus=75`). If busy, it sets
  `summary.daemon.upgrade_pending` ("update installed; restarts when idle").
  `fbittorrent restart` drains and restarts now.
- Why: during development `~/.local/bin/fbittorrent` is replaced (by `install.sh`'s `mv`; a `cp`
  onto a running binary fails with `ETXTBSY`) while a long-lived daemon
  keeps running old code, and the plugin is a symlink into the repository that hot-reloads on every
  save (`~/.config/omarchy/plugins/oliver.torrents -> …/fbittorrent/plugin/oliver.torrents`), so
  both directions of skew are real [FO §7.2].
- Feature skew (plugin newer than daemon, or older) is handled by `caps` (§3.4.3): a client hides
  what the daemon cannot serve, and the daemon rejects unknown ops with
  `unknown_op` so a newer client fails loudly on exactly the feature that is missing, not later.

#### 3.2.13 `status` never starts the daemon

`status` sends `hello` to `api.sock` (and, during the compat period, probes `engine.sock`). When
nothing answers, it reads the ledger and prints
`{"ok": true, "running": false, "app_open": false, "backend": "stopped", "total": N}` with exit
0. Today the closed case prints `"ok": false` with exit 0 (`src/main.rs:407-415`), which the reality
check calls an envelope inconsistency [RC §3 #21]. The only consumer of the old shape is the
v0.1 widget, which keeps polling `status --json` until P2-03 replaces it; P0-07 changes its
`online` test to `status.ok === true && status.running !== false` so it keeps hiding under the
new shape during the M1a weeks. Contract 1.2.0 documents the change (P1-11).

#### 3.2.14 Without a systemd user manager

Rare (a TTY login without `pam_systemd`, a container; never the owner's desktop or an SSH login,
where `pam_systemd` starts the user manager). Detected when `systemctl --user show-environment`
fails. A command that would start the daemon exits 4 with the hint `fbittorrent serve`; the owner
runs the daemon in a terminal or tmux window and every client attaches to it. There is no
self-daemonising path (double fork, log rotation, a remembered log path), because the only
environment that would use it is one the owner does not have, and the test harness runs `serve`
as a foreground child (P1-07, `daemon.py --no-systemd`).

#### 3.2.15 Logging

Each log line on stderr is `level source message key=value…`; journald adds timestamps under
systemd. The same entries enter an in-memory ring of 2,000 entries that the `log` topic serves
(P1-14). `engine/last-error.txt` (`src/engine.rs:484-487`) is retired once the ring exists.
Sources are an open enum: `daemon`, `engine`, `queue`, `api`, `search`, `rss`, `shows`, `imdb`,
`notify`, `network`, `watch` (readiness), `clipboard`.

### 3.3 Module layout

```
src/main.rs                    CLI definitions and dispatch only
src/cli/shapes.rs              today's CLI JSON shapes built from A.9 objects (P3-07)
src/cli/fallback.rs            the in-process path for search, rss, shows, imdb when the daemon is down (P3-07)
src/daemon/mod.rs              run_serve(): lock, signals, sd_notify, supervisor, shutdown
src/daemon/lifecycle.rs        Lifecycle; replaces QUIT_REQUESTED, RESTART, CLOSE, EXPECTED
src/daemon/slot.rs             EngineSlot and engine_epoch
src/daemon/policy.rs           busy table evaluation and the idle countdown
src/daemon/upgrade.rs          binary replacement detection
src/daemon/notify.rs           notify-send wrapper with actions (P1-13)
src/daemon/inhibit.rs          systemd-inhibit holder (P6-09)
src/daemon/sdnotify.rs         READY/STATUS/STOPPING datagrams, std only
src/daemon/watch.rs            watch-readiness derivation (P5-05)
src/daemon/clipboard.rs        opt-in clipboard watcher (P6-13)
src/daemon/log.rs              the log ring and the `log` topic feed (P1-14)
src/api/mod.rs                 protocol types (serde): requests, responses, events, objects, errors
src/api/server.rs              NDJSON listener, connections, dispatch pool, peer-cred check
src/api/bus.rs                 topics, subscriptions, snapshots, deltas, revisions, heartbeats
src/api/mutations.rs           the ref ring, pending results, mutation.result (P1-09)
src/api/ops/mod.rs             api::ops::TABLE: every operation with args, mutating, idempotent, since (P1-18)
src/api/ops/{app,transfer,torrents,search,rss,shows,imdb}.rs   handlers per namespace
src/api/client.rs              blocking client for the CLI and TUI; ensure_daemon()
src/api/legacy.rs              engine.sock 4-byte adapter onto typed ops (until v0.4.0)
src/engine/mod.rs              AppEngine start and drop (from src/engine.rs)
src/engine/ledger.rs           Record, read, save, fsync
src/engine/queue.rs            reconcile_queue and limits
src/engine/ops.rs              EngineOp enum and execute(); replaces string routes
src/engine/view.rs             TorrentView (normalized) and the one legacy qBittorrent mapping
src/engine/parked.rs           ParkedLedger (from dispatch_parked_in)
src/engine/legacy_import.rs    migrate_legacy (deleted in P4-09)
contracts/ipc/v1/*.ndjson      fixtures shared by the Rust and QML tests
docs/IPC.md                    the contract and its changelog
```

`rss.rs`, `shows.rs`, `search.rs`, `imdb.rs` and `browse.rs` stay where they are unless P1-02
finds that moving them under `src/features/` costs little; the decision is by diff size, because
these files carry the owner's uncommitted work and churn makes review harder.

### 3.4 API v1

The complete message catalogue is Appendix A; the CLI mapping is Appendix F. This section fixes
the rules.

#### 3.4.1 Transport and framing

- Unix stream socket `api.sock` in the runtime directory.
- One UTF-8 JSON object per line, terminated by `\n`. Client-to-server lines are at most 64 KiB;
  server-to-client lines at most 1 MiB. A longer line gets `invalid_message` and the connection is
  closed. A line that is not a JSON object (an array, a scalar, invalid UTF-8) gets the same. The
  1 MiB bound is what the plugin's `SplitParser` relies on (§4.4 rule 5), so it is a hard bound on
  every pushed frame (snapshots are chunked, §3.4.7) and on every result the plugin asks for; the
  two results that grow with the library, `torrents.list` and `torrents.files`, are not capped
  (the CLI and TUI read them with `BufRead`) and the plugin never calls them (it subscribes;
  `plugin/tests/lint.sh` rejects the two op names in `plugin/`, P2-05).
- No binary payloads. A `.torrent` file is passed by path (same user, same machine); the daemon
  opens it with `O_NOFOLLOW`, requires a regular file of at most 16 MiB that starts with `d`
  (bencoded), as the CLI checks today (`src/main.rs:518-523`). Today's IPC sends file bytes as a
  JSON number array, about 3.5 times the size [RC §7].
- Why NDJSON: `Quickshell.Io.Socket` with `SplitParser` reads lines natively and cannot practically
  parse a binary length prefix (`/usr/lib/qt6/qml/Quickshell/Io/*.qmltypes`: `Socket`, `SplitParser`;
  [FO §2.6]); the CLI and TUI read lines with `BufRead`. Why caps: a bounded line protects the
  desktop shell from unbounded memory, which the marketplace reviewer required of ucmz851
  [OR §6 gotcha 7]. 1 MiB fits a 256-item snapshot chunk with room to spare (a `Torrent` with a
  1,024-character name serialises under 2 KiB).

#### 3.4.2 Envelope

```
request   {"id": 7, "op": "torrents.stop", "args": {"hashes": ["3f2a…"]}, "ref": "bar-1729-3"}
success   {"id": 7, "ok": true, "result": {"state": "done"}}
failure   {"id": 7, "ok": false, "error": {"code": "not_found", "message": "no torrent 3f2a…",
           "hint": "fbittorrent list --json", "retryable": false, "details": {"hash": "3f2a…"}}}
event     {"ev": "delta", "sub": 2, "rev": 1043, "topic": "torrents", "changed": {…}}
```

- `id` is a positive integer, unique per connection while in flight. Responses may arrive out of
  order and are matched by `id`. Events never carry an `id`. Reusing an in-flight `id` is
  `invalid_args` with `details.key = "id"`.
- `ref` is an optional idempotency key of at most 64 ASCII characters (§3.4.8).
- Error messages quote at most 80 characters of user input and never echo whole frames [OR §2.1].
- Every error carries `hint` when there is a copy-pasteable next step (the CLI prints it after
  "try:", `src/agent.rs:121`), `retryable` always, and `details` when a field can be named.

#### 3.4.3 Handshake and capability negotiation

The first line must be `hello` within 5 s; otherwise the server answers `hello_required` and
closes.

```
→ {"id":1,"op":"hello","args":{"proto":1,
     "client":{"name":"oliver.torrents","version":"0.2.0","kind":"bar"},"interactive":false}}
← {"id":1,"ok":true,"result":{"proto":1,
     "server":{"version":"0.2.0","build":"564fa18"},"engine":"librqbit 9.0.1",
     "instance":"7d0c2f9e-…","engine_epoch":1,"state":"running",
     "caps":["torrents","files","subscribe","mutations","log","settings"],
     "limits":{"line_in":65536,"line_out":1048576,"inflight":8,"subs":16,"hashes":256},
     "policy":{"idle_exit_secs":300}}}
```

`client.kind` is one of `cli` (a human at the CLI), `agent` (the CLI with `--json`, so the
counters below tell agent use from human use), `tui`, `bar` (the plugin's one connection, which
also serves the dashboard, §4.4 rule 8; there is no `dashboard` kind) and `test`. `interactive` can change
later through `app.client {interactive}`, because the plugin's single socket serves the bar and
the dashboard (§4.4). The daemon counts connections per kind in memory (`app.info.clients`, reset at start); nothing leaves the
machine.

`caps` is the negotiation: a closed list of feature groups the server implements, each tied to
the item that adds it. Clients hide or disable UI for a missing cap instead of calling and
failing. A client that needs a cap the server lacks (a plugin newer than the daemon) shows the
version-skew banner with "restart fbittorrent" (§3.2.12).

| Cap | Operations and topics it covers | Since |
|---|---|---|
| `torrents` | `torrents.list/get/add/start/stop/remove/open_target`, topic `torrents`, `summary` | P1-06 |
| `files` | `torrents.files/set_files`, topic `torrent:<h>.files` | P1-06 |
| `subscribe` | `subscribe/unsubscribe`, snapshots and deltas | P1-08 |
| `mutations` | `ref` replay, `mutation.result`, pending states | P1-09 |
| `log` | `app.log`, topic `log` | P1-14 |
| `settings` | `app.settings.get/set`, topic `settings` | P3-05 |
| `staging` | `stage: true`, `torrents.commit/discard` | P3-06 |
| `search` | `search.*`, topic `search:<id>` | P3-01 |
| `rss` | `rss.*`, topics `rss.feeds`, `rss.items:<feed>` | P3-02 |
| `shows` | `shows.*`, topics `shows`, `shows.releases:*` | P3-03 |
| `imdb` | `imdb.lookup` | P3-04 |
| `peers` | `torrents.peers`, topic `torrent:<h>.peers`, `seeds` field | P5-01 |
| `recheck` | `torrents.recheck` | P5-03 |
| `limits` | `transfer.set_limits/set_alt`, `summary.alt`, `summary.limits` | P5-04 |
| `watch` | `playable` fields, `torrent.playable` event, `wait --until playable` | P5-05 |
| `trackers` | `torrents.trackers/reannounce`, topic `torrent:<h>.trackers` | P6-01 |
| `pieces` | `torrents.pieces`, topic `torrent:<h>.pieces` | P6-02 |
| `history` | `summary.history`, `app.stats {history}` | P6-03, P6-04 |
| `queue` | `torrents.force_start/queue_move`, `queue_pos`, `force` | P6-05 |
| `origin` | `origin` field, `{restore: hash}` undo source | P6-08 |
| `clipboard` | `clipboard.magnet` event, `app.clipboard.dismiss` | P6-13 |
| `torznab` | provider kind `torznab` | P7-01 |
| `move`, `rename`, `tracker_edit`, `download_limit`, `deferred_add` | the P7 operations | P7-02..P7-09 |

#### 3.4.4 Versioning rules

- `proto` is the major version. The server speaks exactly its major (1) and answers anything else
  with `proto_unsupported`.
- Additive changes within a major are: operations, topics, caps, optional result and event fields, and new
  values in enums the contract marks as open (`reason`, log `source`, `origin`,
  `busy_reasons`). Clients ignore unknown fields and treat unknown open-enum values as "other".
- Within a major the server never removes or renames a field, changes a type or unit, changes the
  meaning of a value, or makes an optional argument required.
- Input is strict: an unknown operation is `unknown_op`; an unknown argument key or a wrong type is
  `invalid_args` naming the key. Output is tolerant. Why both: typos in agent or plugin code fail
  loudly, while the server can still grow fields without breaking older clients (the qBittorrent
  `webapiVersion` model [FO §5]). Sprout forbids adding fields to existing shapes [OR §2.1]; that
  is stricter than needed when one repository owns every client.
- `docs/IPC.md` carries a changelog: one entry per cap or field added, naming the item that added
  it. There is no minor version number; `caps` is the only feature negotiation (one repository owns
  every client, and a number nobody branches on is ceremony).

#### 3.4.5 Errors

Error codes, their meaning and their CLI exit codes are in Appendix A.3. The CLI keeps its exit
code dictionary (`src/agent.rs:12-29`, `README.md:134-135`) and maps API errors onto it, so
agents see no change. The mapping is total: every API code has exactly one CLI code, and every
CLI code 1..8 is reachable (Appendix F lists the pairs).

#### 3.4.6 Operations overview

Namespaces: `app`, `transfer`, `torrents`, `search`, `rss`, `shows`, `imdb`, plus `subscribe` and
`unsubscribe`. The operations are listed once, in A.4 to A.6 (and, after P1-05, in `docs/IPC.md`);
this section does not repeat them, so the two cannot drift.

#### 3.4.7 Subscriptions, snapshots, deltas and revisions

- `subscribe {topics, interval_ms}` returns `{sub}`. For each keyed topic the server then sends
  `snapshot.begin {sub, topic, rev, count, reason}`, one or more `snapshot.chunk {sub, topic,
  rev, items}` (at most 256 items each), and `snapshot.end {sub, topic, rev}`. After that it sends
  `delta {sub, topic, rev, changed: {key: {field: value}}, added: [object], removed: [key]}`.
  In `changed`, a field set to `null` means the value became unknown and the client deletes it;
  full objects never contain `null` (§3.4.9).
- **Atomicity.** All chunks of one snapshot are taken from one consistent state under the ledger
  lock; for a subscription no `delta` or `state` frame is sent between its `snapshot.begin` and
  `snapshot.end`; changes that happen meanwhile arrive as the first `delta` after `snapshot.end`.
  `Store.js` and `store.rs` may therefore apply chunks as they arrive and treat `snapshot.end`
  as "consistent now".
- Keyed topics and their keys: `torrents` (info hash), `torrent:<hash>.files` (file index),
  `torrent:<hash>.peers` (peer address), `torrent:<hash>.trackers` (URL), `rss.feeds` (feed id),
  `rss.items:<feed>` (item id), `shows` (IMDb id), `shows.releases:<imdb>|inbox` (release key),
  `search:<id>` (result key), `log` (seq, append-only).
- Unkeyed small topics (`summary`, `network`, `settings`, `stats`, `torrent:<hash>.pieces`) send
  `state {sub, topic, rev, data}` with the whole object when it changes, at most once per
  interval.
- `interval_ms` is clamped to 250..10000 (default 1000). Structural changes (added, removed, state
  change, completion, playable, error, network change) go out immediately; rate and progress
  changes are coalesced to the interval. The coalesced set is closed: `progress`,
  `completed_bytes`, `uploaded_bytes`, `dl_bps`, `ul_bps`, `eta_s`, `peers`, `seeds`, `ratio`,
  `playable.buffered_s`, `playable.finishes_in_s`, every `Summary` rate and count and every
  `Peer`/`Tracker` counter; a change to any other field of any object is structural (so a
  `playable.ready` or `stall_free` flip, a `name` change or a new `reason` go out at once).
  "Immediately" is event-driven, not tick-driven: the bus
  waits on each live torrent's `ManagedTorrent::state_changed()` (a fork accessor that selects over
  the existing `pub(crate) state_change_notify`, `vendor/rqbit/librqbit/src/torrent_state/mod.rs:212`,
  notified on every transition at `:333,412-460,518-525`, and over the live state's
  `finished_notify`, `torrent_state/live/mod.rs:201,910`, which is the only signal completion
  sends) and on the ledger's change counter, and
  emits the structural frame on wake; the per-tick snapshot only carries rates and progress. Why:
  rates are noisy; state changes should feel instant [FO §7.2], and a 1,000 ms default tick would
  make "pausing…" linger for a second.
- **Revisions.** `rev` is a per-subscription counter that increases by exactly one on every frame
  the server sends for that subscription (`snapshot.begin`, each `snapshot.chunk`,
  `snapshot.end`, `delta`, `state`). The client stores the last `rev` per subscription and checks
  `incoming == last + 1`. A gap means a frame was lost; the client sends `unsubscribe` then
  `subscribe` for that topic and rebuilds from the new snapshot. On one connection gaps cannot
  happen, because the server closes rather than skips (next point); the check is a safety net
  and the fixture `subscribe-gap.ndjson` exercises it by feeding a client a skipped `rev`.
- **Backpressure and slow clients.** Each connection has a bounded outgoing queue of 512 frames.
  The bus never blocks the engine: a frame for a full queue is not enqueued; instead the server
  marks the connection `slow`, tries once to write `bye {reason: "slow_client"}` with a 1 s
  deadline, closes the socket and logs "slow client <kind> <name>". The client reconnects with
  backoff and receives a fresh snapshot. Why close rather than skip: skipped deltas leave the
  client silently wrong, while a reconnect is cheap and correct (Sprout does the same with a
  256-frame queue [OR §2.1]). Why 512: the bar subscribes to `summary` at 5 s and the dashboard
  to at most 8 topics at 1 s; even a frozen shell for 60 s (the Quickshell reload window) fills
  about 8 × 60 = 480 frames, so a healthy client never trips it, and a wedged one is cut within a
  minute. Per-topic coalescing means at most one pending `delta` per keyed topic per interval,
  so bursts are bounded by topic count, not by torrent count.
- Liveness: a subscribed connection with no frame for 15 s receives `heartbeat {ts}`. Clients treat
  45 s of silence as a dead connection. The server applies **no** idle-read deadline to subscribed
  connections; Sprout's 30 s read deadline silently dropped listen-only clients [OR §6 gotcha 1].
- Engine restart: `engine.changed {epoch, state}`, then a new snapshot with
  `reason: "engine_restart"` on each torrent-like subscription. The connection stays open.
- Detail topics (`torrent:<hash>.*`) exist only while a details view is visible and are closed
  with `unsubscribe`. Per-peer and per-piece data are the expensive part; only the visible torrent
  pays for them. The server releases a detail topic's sampling when its last subscriber leaves.
- Snapshot ordering: keyed snapshots are sent in the topic's natural order (torrents: `added_at`
  descending; files: index; releases: newest first) so a client that renders chunks as they
  arrive shows a sensible list before `snapshot.end`.

#### 3.4.8 Mutations: refs, pending and confirmed

- Every mutating operation accepts `ref`. The daemon keeps the last 256 refs per `client.name`,
  each keyed by the `(client.name, ref)` pair, per instance, for 10 minutes (P1-09 has the
  eviction and reuse rules). Repeating a ref returns the stored response without
  applying the operation again. Why keyed by client name: two clients that happen to pick the
  same ref must not collide; why per instance: §3.2.11. Clients generate refs as
  `<name>-<start-millis>-<counter>`, and a `client.name` must be unique per process for clients
  that run many at once: the CLI and TUI use `cli-<pid>` and `tui-<pid>`; the plugin keeps the
  fixed `oliver.torrents` because one shell process reconnects under the same name and must be
  able to resend a pending ref after a reconnect (§4.4 rule 6). A unit test runs two clients
  with identical refs and distinct names and asserts both mutations apply.
- Synchronous operations (start, stop, set_seed, set_files, live settings, read markers) answer
  after the durable intent is saved and the engine has applied it: `{"state": "done"}`. That is
  already how dispatch works: save first, then reconcile the queue (`src/engine.rs:753-818`).
- Long operations (add while metadata resolves, recheck, remove with files, move, restart-class
  settings) answer `{"state": "pending", "ref": "…", "hash": "…"}` and later push
  `mutation.result {ref, outcome, error?, result?}` with `outcome` one of `done`, `failed`,
  `timeout`. A timeout means "unknown", not "failed" [OR §8 item 3]. A client that sent no `ref`
  gets a server-generated one back so it can still match the result.
- UIs render a pending mutation as a verb in progress ("pausing…", "removing…") on a dimmed row.
  They never show the expected end state before a delta confirms it, and never retry a remove on
  their own.
- Destructive intent is explicit: `torrents.remove` requires `delete_files` as a boolean, and a
  missing value is `invalid_args` [OR §8 item 4]. Hashes are an explicit list of at most 256,
  never `all`. The CLI expands `all` itself and still requires `--yes` with `--files`
  (`src/main.rs:583-605`).
- `torrents.add` returns the info hash at once for magnets (from the `xt`) and `.torrent` paths
  (the daemon bencode-decodes the info dictionary before answering, as the restore path already
  parses files, `src/engine.rs:206-207`); only `{url}` sources are `pending` until the download
  completes.
  Today the CLI diffs `torrents/info` before and after an add to find the hash, which races with
  concurrent adds (`src/main.rs:525-532`, `550-560`) [RC §7].
- A duplicate add returns `ok: true` with `duplicate: true` and the existing hash; it never
  creates a second record and never changes the existing one's settings.

#### 3.4.9 Unknown values are absent

| Field | Absent when |
|---|---|
| `progress` | parked; metadata unknown |
| `size`, `files` | before metadata |
| `eta_s` | not downloading, or rate is zero (today's sentinel 8640000, `src/engine.rs:1477`, disappears) |
| `seeds` | until P5-01 can count them; always present afterwards for live torrents |
| `peers`, `dl_bps`, `ul_bps` | engine not live for that torrent (parked, starting, dormant) |
| `ratio` | nothing downloaded yet |
| `playable` | not a video, metadata unknown, or the `watch` cap is absent |
| `free_space` | `statvfs` failed or returned a negative value [OR §6 gotcha 6] |
| `rating` (IMDb), `uploaded_at` (search), `size` (RSS item) | the source did not provide it (today RSS shows "0 B" [RC §9 #4]) |

Rust uses `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none")]`. A contract test
walks every fixture object and fails on `-1`, `8640000` or a zero in a field documented as
"absent when unknown". Rates are the one place where 0 is a true value (live and idle).

#### 3.4.10 Limits

| Limit | Value | Why |
|---|---|---|
| Connections | 32 | CLI bursts plus the plugin, a TUI and agents; today 16 one-shot workers (`src/ipc.rs:27`) |
| In-flight requests per connection | 8 | A slow `add` must not block a `stop` on the same socket |
| Dispatch pool | 8 threads, queue of 64; beyond that `busy` (retryable) | Bounded like today's server |
| Subscriptions per connection | 16 | The plugin's one connection carries the bar, the popup and the dashboard: `summary`, `torrents`, `network`, `settings`, `rss.feeds` and `shows` (badges) plus a view topic and up to two detail topics, nine on a bad day; the TUI mirrors the same six plus details (P4-03). 8 would refuse the ninth with `busy`; 16 costs nothing and is still a bound |
| Outgoing queue per connection | 512 frames | §3.4.7 |
| Concurrent search sessions | 4 | Each is one nova2 process plus a `spawn` pool of `min(cpu_count, 4)` workers that run the enabled providers (`vendor/nova3/nova2.py:71-75,262-264`): at most about 5 processes and 150 MB per session |
| Hello deadline | 5 s | Drops clients that never speak |
| Default op timeout | 15 s; `torrents.add` 95 s | Matches today's add timeout (`src/ipc.rs:96-100`) |
| Ref ring | 256 per client name, 10 min | §3.4.8 |
| Log ring | 2,000 entries | §3.2.15 |

#### 3.4.11 Fixtures and contract tests

- `contracts/ipc/v1/<group>.ndjson`, one file per feature group. Each line is one of
  `{"dir":"c2s","frame":{…}}`, `{"dir":"s2c","frame":{…}}`, `{"dir":"expect","model":{…}}` or
  a first line `{"dir":"volatile","paths":["result.instance", …]}` listing values to ignore.
- Rust test 1 (`api::fixtures::roundtrip`): every frame deserializes into the typed enums and
  re-serializes to the same canonical JSON (keys sorted, no whitespace).
- Rust test 2 (`daemon.py::contract_<group>`): a real daemon in an isolated XDG directory receives
  the `c2s` frames; its replies must match the `s2c` shapes, ignoring volatile paths. Each
  mismatch is logged with the JSON pointer of the first differing key.
- QML test (`qs-store.sh`): `Store.js` consumes the `s2c` frames and must produce each `expect`
  model (P2-02).
- Sentinel scan (`api::fixtures::no_sentinels`): §3.4.9.
- Why: Sprout loads its documentation examples in tests so they cannot drift [OR §8 item 1].

### 3.5 The normalized torrent model

One closed `state` enum replaces the three copies of qBittorrent state strings (§1.7):

| `state` | Meaning | Derived from (`src/engine.rs:1396-1440` today) |
|---|---|---|
| `metadata` | Magnet resolving | handle without metadata |
| `checking` | Hashing existing data | `Initializing { paused: false }`, or any `Initializing` while the daemon holds the hash in its `rechecking` set (P5-03) |
| `downloading` | Live, incomplete, receiving | `Live`, not finished, rate > 0 |
| `stalled` | Live, incomplete, nothing received | `Live`, not finished, rate = 0 |
| `queued` | Wants to run; held by the queue limit | handle paused, record not paused, not finished, no error |
| `paused` | Paused by the user (durable) | `record.paused` |
| `done` | Complete; stopped because seeding is off | finished and stop-on-complete (`ratio_limit == 0.0`, `src/engine.rs:90-92`) |
| `seeding` | Complete; seeding allowed; live | finished, not stop-on-complete, `Live` |
| `waiting_vpn` | Engine parked | parked slot |
| `error` | Engine error; `error` has the message | `Error` |

Two companions: `reason` (open enum: `user`, `queue_full`, `complete_no_seed`, `vpn_down`,
`restore_failed`, `metadata_timeout`, …) explains why a torrent is not running, as Fragments shows
a stop reason (`Fragments src/backend/connection_manager.rs:50-55`; [FO §5]); `checked: false`
marks a completed torrent restored without hashing (P1-16). The legacy adapter maps the
normalized model to qBittorrent strings once, in `src/engine/view.rs`, for old clients. All
fields are listed in Appendix A.9. Every combination of rqbit state and record flags maps to
exactly one `state` (table-driven test in P1-02).

Watch-readiness (P5-05) adds `playable` to the model: `{file: index, buffered_s?, ready: bool,
stall_free?: bool, finishes_in_s?, acked: bool}` for the largest video file (by extension list
`mkv mp4 webm avi mov m4v ts`), present only when the `watch` cap exists and metadata is known.
`ready` is the DL-26 threshold and fires the notification once; it is deliberately early (32 MiB
is about 30 s of 1080p video), so on a link slower than the bitrate the player would stall a
minute later and the owner would learn to ignore the notification. `stall_free` is therefore
recomputed every second while downloading from the prefix, the file size, `dl_bps` and the
container's duration (when the header gives one), and is what the rows show: "Ready to watch ·
4 min buffered" when `stall_free` is true, or unknown with `buffered_s ≥ 120`; "Playable · may
stall (finishes in 18 min)" otherwise. `o` works in both. Without a duration the fallback is
`dl_bps ≥ file_size / 2 h`, labelled as an estimate in the details line.

### 3.6 Migration from the 4-byte IPC

| Version | `api.sock` | `engine.sock` (4-byte) | CLI | Plugin | TUI |
|---|---|---|---|---|---|
| v0.1.x (today) | absent | engine inside the TUI | 4-byte | spawns `status --json` | owns the engine |
| v0.2.0 (M1) | daemon | daemon, compat adapter | API v1 | API v1 popup | client over `engine.sock` (P1-15) |
| v0.3.0 (M2) | daemon | daemon, compat adapter | API v1 | API v1 popup and dashboard | moves to API v1 in P4-03 |
| v0.4.0 (M3) | daemon | removed (P4-09) | API v1 | API v1 | API v1 |

A new CLI that finds only an old, TUI-hosted `engine.sock` (no `api.sock`) still reads through it
but refuses writes with "an older fbittorrent app is running; quit it with Q and retry". Why: mixing
a new client with an old engine during the upgrade is exactly the skew a daemon introduces; reads
are harmless, writes against an unknown engine are not. This can happen once, during the first
upgrade, and is checked by hand then (M1a receipt); the compat adapter's consumer is the TUI of
P1-15 and `native.py` tests it there. The v0.1 TUI owns its engine and never attaches
(`src/main.rs:787`), so after installing v0.2 the only TUI is the new one.

### 3.7 Search, RSS, Shows and IMDb move into the daemon (assumption A4)

| Feature | Today | After P3 | CLI while the daemon is down |
|---|---|---|---|
| Search jobs | calling process (`src/qbt.rs:609-627`; registry `src/search.rs:723-738`) | daemon sessions; results stream on `search:<id>` | in-process, as today |
| Provider preferences | file plus lock (`src/search.rs:254-256,399`) | daemon ops | in-process |
| RSS | calling process (`src/qbt.rs:547-585`); `rss.json` with lock and atomic rename (`src/rss.rs:69-71,93-100`) | daemon ops and topics | in-process |
| Shows | CLI (`src/main.rs:1022+`) and TUI; `shows.json` with lock and three-way merge on save (`src/shows.rs:246-262`) | daemon ops and topics | in-process |
| IMDb | calling process; each process loads the 28.7 MB ratings file (`src/imdb.rs:117-183`) | daemon: index loaded lazily once, posters cached as files (`src/imdb.rs:101-115` already caches under `~/.cache/fbittorrent/posters`); `imdb.lookup` returns `poster_path` for `file://` loading | in-process |
| Browse lists | TUI (`src/browse.rs:31-44`) | `search.browse` | in-process |

- **Concurrency.** The stores already lock and merge on save, so a daemon and an in-process CLI can
  run at once. The daemon re-reads a store when its mtime changed before serving it, so CLI edits
  appear in the UIs.
- **Scheduling.** "On app open" becomes "when a client becomes interactive" (a TUI says hello, or
  the dashboard window opens): refresh each feed once if the last refresh is older than 30 minutes;
  check each show whose last check is older than 30 minutes (today's rule, `README.md:36`,
  `src/tui/mod.rs:1664-1666`; `Show::is_stale`, `src/shows.rs:115`). No periodic background
  polling while nobody looks, which keeps an idle daemon off the network.
- **Search sessions** belong to the connection that started them and stop 30 s after it
  disconnects. Providers run as today (bounded capture, process-group kill, `PR_SET_PDEATHSIG`,
  `src/search.rs:583-688`), now as children of the daemon. A session's results are retained for the
  30 s grace so a dashboard that reloaded (Quickshell hot reload) can resubscribe and keep them.
- **Dedup (fixes D15).** Key: the btih from the magnet or the `.torrent`; when a provider gives only
  a page URL, the normalized (title, size) pair. Keep the best-seeded entry and list every provider
  that returned it. Today dedup falls back to the full URL, so duplicates remain
  (`src/tui/mod.rs:2979-2997`).
- **Mojibake repair.** If a name round-trips as UTF-8 that was decoded as Latin-1
  ("è¾›ç‰¹å°”"), re-decode it; otherwise leave it unchanged. A unit test covers the captured sample
  from `screens/07-search-sintel-16s.txt`. Why this test and no other: re-encoding the string as
  Latin-1 and decoding the bytes as UTF-8 succeeds only when every byte sequence is a valid
  multibyte UTF-8 character, which a real Latin-1 title ("Amélie", "Straße") almost never is, so
  the repair cannot damage a name that was right; and it is reversible, so a false positive is
  recoverable from the provider's raw bytes, which the daemon keeps for the session.

### 3.8 Notifications, log and sleep inhibitor

- **Notifier.** A queue drained by at most 4 threads (further notifications wait; the same
  `(kind, hash)` pending twice is collapsed; `notify-send --wait` blocks until the notification
  closes, which can be minutes for `critical`) runs, per notification,
  `notify-send -a fbittorrent -i fbittorrent -A open=Open -A folder="Show in folder" "Download complete" "<name>"`.
  On this machine `notify-send --help` lists `-A/--action` ("Implies --wait … The name of the
  action is output to stdout", verified 2026-10-18); the daemon reads the chosen action and runs
  `xdg-open` on the file or folder. Events and their actions:

  | Event | Title | Actions | Item |
  |---|---|---|---|
  | added (handler; and the CLI when stdout is a TTY or `--notify` is given; agents, `--json` and `--quiet` get none, so ten `add` calls do not make ten toasts) | "Added" | `undo` (remove, keep files), `window` (open dashboard); the daemon posts these only for `origin: handler`, and the CLI's TTY toast is plain, without actions (P1-13, Q33) | P1-13 |
  | add failed | "Could not add" | `log` | P1-13 |
  | completed | "Download complete" | `default` and `open` (play; `notify-send` invokes `default` on a click of the body), `folder` | P1-13 |
  | ready to watch | "Ready to watch" | `default` and `open` (play) | P5-05 |
  | disk almost full | "Disk almost full" | `window` | P5-07 |
  | VPN waiting / back | "Waiting for ProtonVPN" / "VPN back" | none | P1-13 |
  | new releases | "3 new episodes" | `window` (Shows view) | P6-10 |
  | clipboard magnet (opt-in) | "Magnet on clipboard: <name>" | `add`, `dismiss` | P6-13 |

  Bodies are escaped for markup (`&`, `<`). `notify = false` disables all of it (`README.md:162`).
  A notification whose action handler process is killed (shell restart) never acts; nothing
  is left half-done because every action is a normal API operation with a `ref`.
- **Icon.** An SVG glyph `assets/fbittorrent.svg`, monochrome, installed by `setup` to
  `~/.local/share/icons/hicolor/scalable/apps/`. It replaces the borrowed `qbittorrent` icon in the
  desktop entries (`src/setup.rs:112-114`) and in `notify` (`src/main.rs:1296-1300`). The glyph
  is the same shape as the bar's download glyph so the notification and the bar read as one app.
- **Log.** A ring of 2,000 entries `{seq, ts, level, source, hash?, message}`; `app.log` and the
  `log` topic serve it; journald keeps the long history.
- **Sleep inhibitor (P6-09).** While any torrent downloads, the daemon holds
  `systemd-inhibit --what=sleep --who=fbittorrent --why=Downloading --mode=block sleep infinity` as a
  child and kills it when downloads stop. `/usr/bin/systemd-inhibit` exists here (checked). A child
  process avoids adding a D-Bus library to a binary that has no async runtime [FO §4.2].

### 3.9 Security model

- **Same user only.** Validated 0700 runtime directory, 0600 sockets, `SO_PEERCRED` uid check, no
  TCP listener, no WebUI.
- **Bounded input.** Line caps, strict argument types, at most 256 hashes per call, names truncated
  to 1,024 characters in list frames (full name in `torrents.get`), `.torrent` by path only with the
  checks in §3.4.1.
- **Files.** Every store is 0600 and written through temp file, fsync, rename and directory fsync
  (the existing pattern in `src/config.rs` and `src/shows.rs:246-262`). A credential file (Torznab
  keys, P7-01) is additionally opened with `O_NOFOLLOW` and refused, not repaired, when its mode is
  wrong [OR §6 gotcha 7].
- **Processes.** Arguments are always argv arrays; the only shell string is the Omarchy launcher
  boundary, which is already quoted (`src/backend.rs:62-71`). QML uses `Util.execArgv`
  (`/usr/share/omarchy/shell/Commons/Util.qml:77-79`) for anything built from data, never
  `execDetached` with a string.
- **Payload deletion** stays anchored to the torrent's output directory and refuses symlinked
  descendants [EE "Ownership and durable state"].
- **Notifications** escape markup; no secret ever appears in argv.
- **Clipboard (P6-13)** is read only when the setting is on; the daemon keeps nothing but the
  last magnet's hash (to avoid repeat offers) and never logs clipboard text.

### 3.10 Threads and performance

- The normal dependency graph has no Tokio, hyper or reqwest today [RC §3 #24]. The API server
  uses std threads, as the current IPC server does (`src/ipc.rs:174-263`).
- Threads: one listener; two per connection (reader, writer); a dispatch pool of 8; one bus; one
  supervisor; one signal thread; short-lived notifier threads. Engine operations still enter the
  Asupersync runtime with `block_on` from dispatch threads (`src/engine.rs:577-583`), the pattern
  the IPC module documents as safe against pool deadlock (`src/ipc.rs:1-8`).
- The bus takes one `torrents` snapshot per tick at the smallest active interval and diffs it per
  subscription: O(torrents) stats reads per tick. Watch-readiness (P5-05) reads the have-bitfield
  of at most the live, incomplete torrents once per second.
- **Budget by measurement, not by guess.** P1-08 measures daemon CPU and RSS with
  `resources.py` for 0, 1 and 8 subscribers and for 10 and 100 torrents. Acceptance: one bar
  subscription at 5,000 ms costs no more CPU than today's `status --json` every 10 s (4 to 5 ms
  of wall time per run [FO §1.4]). The numbers go to `docs/validation/<date>/daemon-budget.md`
  and become the regression baseline that P0-06's resource gate checks.

---
## 4. Quickshell plugin design (`oliver.torrents` evolves)

This section is the visual and interaction specification for the bar icon, the popup and the
dashboard window. Everything in it is expressed in the shell's own tokens so that a theme switch
changes every value at once; every token named here exists in `/usr/share/omarchy/shell/Commons/`
at the cited line. Where a value has no token (the progress track alpha, the cursor bar width)
the section says so and names the one local constant.

### 4.1 Structure and manifest

```
plugin/oliver.torrents/
  manifest.json      kinds ["service","bar-widget","panel"]
  Service.qml        service singleton: the one API connection, the Store, actions
  ApiClient.js       .pragma library: framing, request ids, pending map, timeouts, backoff
  Store.js           .pragma library: reducer for snapshot/chunk/end/delta/state; counts, filters, sort
  Model.js           formatting only: sizes, rates, ETA, state → glyph and word (§4.3.4)
  Glyphs.js          every Nerd Font glyph by role, one place (§4.2.4)
  BarWidget.qml      bar-widget: BarIconButton + KeyboardPanel popup (today's Panel.qml, rewritten)
  Dashboard.qml      panel kind: owns the FloatingWindow and routes views
  views/             TorrentsView, DetailsPane, SearchView, BrowseGrid, ShowsView, RssView,
                     SettingsSheet, AddSheet, RemoveConfirm, LogView, HelpOverlay
  components/        TorrentTable, PopupRow, ProgressRule, StateText, PiecesBar, Sparkline,
                     PosterCard, Banner, EmptyState, PaneFrame, KeyHint
  (tests live in plugin/tests/, outside this folder: it is symlinked into ~/.config/omarchy/plugins/
   and any file change under it reloads the shell, §4.15)
```

```json
{
  "schemaVersion": 1,
  "id": "oliver.torrents",
  "name": "Torrents",
  "version": "0.2.0",
  "author": "Oliver",
  "license": "MIT",
  "description": "fbittorrent in the Omarchy bar: status, quick actions and a full window.",
  "kinds": ["service", "bar-widget", "panel"],
  "entryPoints": { "service": "Service.qml", "barWidget": "BarWidget.qml", "panel": "Dashboard.qml" },
  "barWidget": {
    "displayName": "Torrents",
    "category": "Network",
    "allowMultiple": false,
    "defaultSection": "right",
    "defaults": { "binary": "fbittorrent", "showWhenStopped": false, "showRates": false,
                  "middleClick": "dashboard",
                  "tuiCommand": "omarchy-launch-or-focus-tui --app-id=org.omarchy.fbittorrent fbittorrent tui" },
    "schema": [ "… one entry per default, as today (manifest.json:23-28) …" ]
  }
}
```

- The entry-point keys are verified against the installed shell: services load from
  `entryPoints.service` (`/usr/share/omarchy/shell/shell.qml:931`), bar widgets from
  `entryPoints.barWidget` (`:1453`), other kinds from the key named like the kind (`:1333`), and
  `entryPointUrl` keeps every path inside the plugin folder
  (`/usr/share/omarchy/shell/services/PluginRegistry.qml:118-133`).
- Summon routing (verified, A3): a plugin with both `bar-widget` and `panel` is excluded from
  `isBarWidgetPanelPlugin` (`shell.qml:1161-1172`), so `omarchy-shell shell summon oliver.torrents
  '<json>'` opens the panel (the dashboard), while the popup keeps its own IPC target
  `oliver.torrents` (`plugin/oliver.torrents/Panel.qml:124-139`) [FO §2.2]. A bar-layout entry
  counts as enabled (`PluginRegistry.qml:148-163`), so `setup` need not add a `plugins[]` entry;
  P3-08 still asserts it with a summon loop.
- The polling settings `refreshIntervalSec` and `idleRefreshIntervalSec` (`manifest.json:18-19`)
  are removed; nothing polls any more.
- The entry-point `Item`s receive `shell`, `manifest` and the registries as injected properties
  (`omarchy-src/agents/skills/shell-dev.md` "Plugin contract"); the dashboard calls
  `shell.hide("oliver.torrents")` through that facade on a user close (§4.6).

### 4.2 Design language

#### 4.2.1 Tokens (all verified in the installed shell)

| Role | Token | Value at base size 12 | Source |
|---|---|---|---|
| Spacing unit | `Style.space(px)` scales with `[spacing] scale` and the font | identity at 12 px | `Commons/Style.qml:215-225` |
| Named spacing | `Style.spacing.{xxs,xs,sm,md,lg,xl,xxl,xxxl,huge}` | 2, 3, 4, 6, 8, 10, 12, 14, 18 | `Style.qml:237-245` |
| Control geometry | `Style.spacing.{controlGap, controlPaddingX, controlPaddingY, controlHeight, popupRowHeight}` | 8, 10, 6, 28, 28 | `Style.qml:247-252` |
| Row geometry | `Style.spacing.{rowGap, rowPaddingX, labelGap}` | 8, 12, 4 | `Style.qml:257-259` |
| Panel geometry | `Style.spacing.{panelGap, panelPadding, popupPadding}` | 14, 18, 14 | `Style.qml:260-262` |
| Type scale | `Style.font.{caption, bodySmall, body, subtitle, title, heading, display, displayLarge}` | 10, 11, 12, 13, 14, 16, 24, 28 | `Style.qml:329-336` |
| Icon sizes | `Style.font.{iconSmall, icon, iconLarge}` | 11, 14, 18 | `Style.qml:338-340` |
| Family | `Style.font.family` (the `monospace` alias; resolves to JetBrainsMono Nerd Font here) | | `Style.qml:271-277`; `fc-match monospace` |
| Palette | `Color.{foreground, background, accent, urgent, muted}` | theme | `Commons/Color.qml:19-23` |
| Surfaces | `Color.popups.{background,text,border}`, `Color.tooltip.*`, `Color.menu.{scrim,selectedBackground,selectedText}` | theme | `Color.qml:84-108` |
| Bar | `bar.foreground`, `bar.urgent`, `bar.fontFamily`, `bar.foregroundAnimationEnabled` | theme | `plugins/bar/Bar.qml:78,84-91`; `Ui/Panel.qml:23` |
| State fills | `Style.{normalFill, hoverFill, selectedFill, pressedFill, selectionFill, selectedAccentFill}` | alpha .04/.08/.18/.22/.35/.18 | `Style.qml:84-89,191-201` |
| Borders | `Border.controlSpec(state, fg, accent)`, `Border.surfaceSpec("popups","border",…)`, `Border.flat(color, w)` | theme | `Commons/Border.qml:217,141,17` |
| Radius | `Style.cornerRadius` (Hyprland `decoration:rounding`) | | `Style.qml:33,464-471` |
| Motion | `Style.reduceMotion`, `Style.duration(ms)` (0 when animations are off) | | `Style.qml:363-367` |
| Dim text | `Model.dim(foreground, background)` in QML and `palette::dim(fg, bg)` in the TUI: **one ladder, defined here and nowhere else.** Rung 1: `Qt.darker(foreground, 1.4)`, which is Qt's HSV darken (value × 100 ÷ 140, hue and saturation unchanged; the TUI reimplements exactly that). Rung 2: `mix(foreground, background, 0.75)`, per channel in 8-bit sRGB, `round(0.75·fg + 0.25·bg)`. Rung 3: the same mix at 0.85. Rung 4: `foreground`. The first rung whose WCAG contrast against `background` reaches 4.5:1 wins. `muted` and `dark_foreground` are never rungs (`dark_foreground` is 2.6:1 on the owner's theme, D16; `muted` reaches 4.5:1 on 3 of 23 installed themes). `contrast.py` computes the reference hex per fixture theme into `contracts/ui/themes/expected.json`; QML and TUI must match it within ±1 per channel (rounding) | theme | `Ui/PanelHero.qml:23`, `Ui/PanelSectionHeader.qml:19` (the 1.4 convention); one fixture (`contracts/ui/themes/`) |
| Unknown marker | the dim colour for "—" (never "∞", §4.2.3) | | omaqbt uses `Color.muted` (`TorrentTable.qml:65-67`); not adopted, because `muted` is a track and border colour (`Commons/Color.qml:168` falls back to `color8`) that reaches 4.5:1 on 3 of 23 installed themes |
| Accent text | `Color.accent` for the ready badge, key letters, the sorted header and the active filter only when `accent` reaches 4.5:1 against `background`; otherwise bold `foreground` (the TUI's §5.5.1 rule) | theme | computed from `colors.toml`: `catppuccin-latte` 4.3:1, `miasma` 3.9:1, `rose-pine` 3.1:1 fail |

Rules that follow from the tokens:

- **No literal colour, size or duration anywhere** in `plugin/`. The lint (P2-05) rejects
  `#rgb…`, named colours, `Qt.rgba(` with numeric channels, bare `duration: <number>` and
  `font.pixelSize: <number>`. The two permitted local constants are the progress-track alpha
  (`Util.alpha(foreground, 0.12)`, there is no track token; Sprout uses `Color.muted` at 0.25,
  `Sprout Panel.qml:1111-1114`) and the cursor bar width `Style.space(3)` (omaqbt
  `TorrentTable.qml:215`).
- **Dim starts at 1.4 and is computed, not fixed.** Today's popup uses `Qt.darker(foreground,
  1.55)` (`Panel.qml:18`); first-party panels use 1.4. On the owner's theme 1.4 gives `#b6b6b6`
  on black (about 10.4:1) and on `catppuccin-latte` 10.2:1, but on `everforest` it is 3.8:1 and on
  `gruvbox` and `tokyo-night` 4.3:1 (computed from their `colors.toml`), so a fixed factor cannot
  pass a 4.5:1 gate. `Model.dim(fg, bg)` therefore walks the ladder in the token table and the
  TUI's `palette.rs` walks the same ladder (§5.5.1); `qs-format.sh` and `palette::tests` share one
  fixture directory, `contracts/ui/themes/` (a copy of each installed theme's `colors.toml` plus
  the owner's, with `expected.json` holding one object per theme file:
  `{"owner": {"foreground": "#ffffff", "background": "#000000", "accent": "#8d8d8d", "dim": "#b6b6b6",
  "dim_rung": 1, "dim_ratio": 10.4, "fg_ratio": 21.0, "accent_ratio": 6.3, "accent_text": false,
  "low_chroma": true}}`, written by `contrast.py`, so the tests do not depend
  on which themes a CI box has installed). P2-08's `contrast.py` computes, for every theme under
  `/usr/share/omarchy/themes/*/colors.toml` and the owner's, the ratio of `foreground`, the
  computed dim and `accent` against `background`, and fails only when the computed dim is below
  4.5:1 or primary text is below 4.5:1 (a theme whose own foreground fails is reported, not fixed,
  because the plugin cannot change it; `rose-pine` is 6.7:1 today, every other installed theme is
  above 7:1). `muted` is reported for information and never used for text.
- **Hover and keyboard cursor are one state** ("hover-cursor", `Style.qml:40-44`): a row under
  the mouse and the row the `j`/`k` cursor is on look identical. Selection (a chosen row, a
  multi-select range) uses `selectedFill`/`selectionFill`. Focus rings appear only on controls
  that take Qt focus (text fields, buttons, toggles) through `Border.controlSpec("focus")`
  (`Ui/Button.qml:81-84`, `Ui/TextField.qml:36-38`).

#### 4.2.2 Layout grid

- The popup is one column `Style.space(380)` wide (every first-party popup's width:
  `panels/network/Panel.qml:1109`, `panels/bluetooth:670`, `panels/tailscale:421`,
  `plugins/agents/Panel.qml:972`; today's 400,
  `Panel.qml:162`, was arbitrary) with
  `Style.spacing.popupPadding` on all sides, sections separated by `PanelSeparator` and
  `Style.spacing.xxl` of vertical rhythm. Height fits content up to `Style.space(560)`.
- The dashboard is three panes on a horizontal axis: sidebar `Style.space(200)`, list (fill),
  details `Style.space(360)` (omaqbt uses 210/380 for a table without STATE, PEERS and ADDED,
  `omaqbt Client.qml:736,763,803`; ours carries three more columns, so the side panes give back
  30 px). Panes are separated by `Style.spacing.panelGap` and framed by `PaneFrame` (a
  `BorderSurface` with `Border.controlSpec("normal")`, padding `Style.spacing.lg`). Each pane has
  a title row of `Style.space(30)` and a body: the list's title row is its column header, the
  details pane's is the torrent name with the tab strip, and the sidebar's is its "All" entry (no
  separate caption, as the §4.6 mockup shows). Chrome with both side panes docked is therefore
  200 + 360 + 2 × 14 + 3 × 16 = **636 px**; with the details as a sheet 200 + 14 + 2 × 16 =
  **246 px**; with the sidebar collapsed to chips and the details as a sheet 1 × 16 = **16 px**.
  §4.3.3's tiers are computed from these three numbers; `qs-dashboard.sh::width_tiers` asserts
  them from the live pane geometry.
- Vertical rhythm inside panes: rows `Style.spacing.popupRowHeight` (28) tall, headers
  `Style.space(30)`, groups separated by `Style.spacing.rowGap`.
- Dialogs (`ConfirmDialog`, the add sheet) are centred cards at most `Style.space(370)` wide for
  confirms (`Ui/ConfirmDialog.qml:52`) and `Style.space(560)` for the add sheet, padding
  `Style.space(18)`.

#### 4.2.3 Typography roles

| Role | Size | Weight and case | Colour |
|---|---|---|---|
| Window/panel title ("Torrents") | `Style.font.title` | bold | foreground (`Ui/PanelHero.qml:58-59`) |
| Hero meta ("2 DOWNLOADING · ↓ 4.2 MB/S") | `Style.font.caption` | bold, uppercase, letterSpacing 1.2 | dim (`PanelHero.qml:95-104`) |
| Section header ("DOWNLOADING") | `Style.font.caption` | bold | dim (`PanelSectionHeader`) |
| Table header ("NAME", "SIZE") | `Style.font.bodySmall` | uppercase, letterSpacing 0.08 em | dim; sorted column foreground (omaqbt `TorrentTable.qml:120-135` uses `muted`; see §4.2.1 why not) |
| Popup row name | `Style.font.body` | regular | foreground; dim when paused/done |
| Popup row meta ("Downloading · 43% · ↓ 3.1 MB/s · 12 min") | `Style.font.caption` | regular | dim |
| Table cell | `Style.font.subtitle` | regular, numerics right-aligned | foreground; "—" dim |
| State word | same as its row | regular | foreground; `error` urgent |
| Key hint ("a add · / search · w window") | `Style.font.caption`, one line, rendered like `PanelSectionHeader` (bold caption, letterSpacing 1.2, no uppercase); **hidden at rest**, shown from the first keyboard key the popup receives until it closes, and always shown in the dashboard | keys in foreground, labels dim; accent only when it passes §4.2.1 | no first-party popup renders a key-hint row (`network`, `bluetooth`, `tailscale`, `agents`: none), which is the one structural tell in the resting popup; hiding it until the keyboard is used keeps the mouse user's popup identical in structure to the Wi-Fi panel (hero, separator, section headers, rows) and keeps discoverability for the keyboard user; omaqbt's table uses body-size hints (`TorrentTable.qml:324-338`); the popup keeps it to one caption line so it reads as section chrome, not a toolbar (Sprout's "card-dashboard" anti-pattern) |
| Empty-state title / body | `Style.font.heading` / `body` | regular | foreground / dim |
| Banner | `Style.font.body` | regular | urgent text on `Util.alpha(urgent, 0.10)` fill with a 0.35 border (Sprout `Dashboard.qml` callouts [OR §3.1]) |
| Tooltip | `Style.font.bodySmall` | regular | `Color.tooltip.text` (via `PanelToolTip`) |

Numbers are formatted once, in `Model.js`, and used by every surface: sizes with one decimal and
binary units up to 100 GiB ("1.4 GiB" in sentences, dialogs, banners and the add sheet; the
compact "1.4G" only in table cells; never "GB" or "MB" for a size, in prose, mockups or fixtures,
so `contracts/ui/format.json` and the captures of P2-08 and P3-16 agree), rates "4.2 MB/s"
(decimal, like the bar's other widgets),
ETA as "12 min", "1 h 05", "2 d" (never seconds above a minute; never "∞": absent ETA renders
"—"), percentages as integers below 99.5 and "99.9" above (so a nearly complete torrent is not
shown as "100%" before `done`), counts with thin spaces above 9,999.

#### 4.2.4 Iconography

Glyphs come from the Nerd Font the bar already uses and live in `Glyphs.js` so a glyph is chosen
once per role. Existing glyphs keep their meaning; new ones are named by their Material Design
Icon name and pinned to codepoints in P2-04, which also renders each one in a test and fails if
`paintedWidth == 0` (a missing glyph renders as a box).

| Role | Glyph today | Source | Note |
|---|---|---|---|
| downloading (bar, hero, row) | `󰇚` | `Panel.qml:145,214`, `Model.js:62` | the brand glyph; also the notification icon shape |
| idle | `󰇚` (dim) | today `󰕒` (`Panel.qml:145`, `Model.js:65`) | changed: one mark, state by weight (§4.5) |
| seeding only | `󰕒` | `Panel.qml:145`, `Model.js:65` | only while something uploads and nothing downloads |
| paused | `󰏤` | `Model.js:61`, `Panel.qml:433` | |
| done | `󰄬` | `Model.js:61` | |
| error | `󰀦` | `Model.js:64` | always paired with the word "Error" |
| add | `󰐕` | `Panel.qml:223` | |
| terminal (TUI) | `󰆍` | `Panel.qml:224` | |
| refresh | `󰑐` | `Panel.qml:225` | no longer a key (pushed state needs no refresh; Appendix B has no `r` in the popup); kept only as the glyph of §4.10's "Refresh" button after an unknown mutation result |
| power (stop the daemon, A1) | `󰐥` | `Panel.qml:226` | |
| play / open | `󰐊` | `Panel.qml:433` | |
| folder | `󰝰` | `Panel.qml:442` | |
| trash | `󰆴` | `Panel.qml:451` | hover colour urgent (`Panel.qml:454`) |
| VPN (waiting / bound) | `󰦝` | the owner's menu uses it for Proton (`omarchy-menu.jsonc:38`) | |
| queued | MDI `clock-outline` | new | |
| metadata | MDI `dots-horizontal` plus the text spinner | new | |
| checking | MDI `sync` | new | |
| ready to watch | MDI `play-circle-outline` | new, P5-05 | the one glyph allowed to use accent colour on a row |
| turtle (alt limits) | MDI `tortoise` | new, P5-04 | shown in the hero and the dashboard top bar when on |
| window (dashboard) | MDI `window-restore` | new | |
| search | MDI `magnify` | new | |
| stalled | MDI `pause-circle-outline`? no: MDI `timer-sand` | new | distinct from paused |

Every state glyph is always followed by its word (§4.3.4). The bar icon is the only place a glyph
stands alone, and there its tooltip carries the word.

### 4.3 Row anatomy

#### 4.3.1 The popup row (two lines, `components/PopupRow.qml`)

```
 ┌ glyph 26 ┬──────────────────────────────── fill ─────────────────────────┬ actions (hover/cursor) ┐
 │  󰇚      │ The Diplomat S04E01 1080p WEB‑DL H264‑GROUP          (name, body, ElideMiddle)      │ 󰏤  󰐊  󰆴              │
 │          │ ▕████████████▌                              ▏ (progress rule, 3 px, radius ½)      │                       │
 │          │ Downloading · 43% · ↓ 3.1 MB/s · 12 min · 41 peers (meta, caption, dim)            │                       │
 └──────────┴────────────────────────────────────────────────────────────────┴───────────────────────┘
   height = content + Style.spacing.rowPaddingX; glyph column Style.space(26); gaps Style.spacing.lg
```

- The name keeps `ElideMiddle` with the full text in a `PanelToolTip` on hover (today's
  `FullTextTip`, `Panel.qml:311-330`) and never scrolls.
- The meta line is built from present fields only, in this order: state word · percent · ↓ rate
  · ETA · peers (and "seeds" after P5-01) · "Ready to watch" badge (P5-05). Absent fields are
  omitted, never zero ("Fetching metadata · 0:12" shows elapsed time instead). The badge's
  "4 min buffered" is `playable.buffered_s` and updates with every delta (coalesced to the
  interval like a rate), so the owner can watch the buffer grow before pressing `o`; when
  `playable.stall_free` is false the badge reads "may stall · finishes in 18 min" (§3.5).
- The progress rule is shown while `progress < 1 || state is live`; its fill colour is
  `foreground` for live states and dim otherwise (`Panel.qml:395-412` today, with the literal
  `Qt.rgba` track replaced by `Util.alpha`).
- Row actions (pause/resume, open, remove) appear on hover or when the keyboard cursor is on the
  row, as in Sprout (`Sprout Panel.qml:1072-1104`); the row keeps its height either way. Why
  hidden otherwise: three buttons per row on a 400 px card is the "card-dashboard" look Sprout's
  rules reject [OR §5]; the keys `p`, `o`, `d` and the footer hints keep the actions discoverable.
- Row states: cursor (hover-cursor fill), pending (name at 0.55 opacity, meta shows the verb,
  Sprout `Panel.qml:1027,1047-1056`), error (`!` glyph in urgent, meta shows the message, no
  colour on the name), done (dim name, no rule), ready to watch (meta badge in accent).

#### 4.3.2 The dashboard row (one line, `components/TorrentTable.qml`)

```
 glyph │ STATE        │ NAME (fill)                        │ SIZE  │ PROGRESS        │  ↓   │  ↑  │ ETA │ PEERS │ RATIO │ ADDED
  26   │ 110          │ ≥ 210                               │ 76    │ 120             │ 80   │ 80  │ 64  │ 56    │ 56    │ 80    (Style.space)
 󰇚    │ Downloading  │ The Diplomat S04E01 1080p WEB‑DL…  │ 1.1 G │ ▕███▌   ▏ 43%   │ 3.1M │ 32K │ 12m │ 41    │ 0.02  │ 12:41
 …     │ Metadata     │ Some.Movie.2026.1080p              │ —     │ ▕       ▏ —     │ —    │ —   │ —   │ 3     │ —     │ 12:40
 󰏤    │ Paused       │ Linux.ISO.x86_64                   │ 1.2 G │ ▕██     ▏ 18%   │ —    │ —   │ —   │ —     │ —     │ Oct 12
 󰄬    │ Done         │ Sintel.2010.1080p                  │ 1.4 G │ ▕███████▏ done  │ —    │ —   │ —   │ —     │ 1.20  │ Oct 3
```

- Widths start from omaqbt's measured set (`omaqbt TorrentTable.qml:48-56`), tightened to the
  compact number formats of §4.2.3 ("1.1G", "3.1M", "12m", "Oct 12"), plus three columns: STATE
  (glyph and word, never hidden), PEERS, ADDED. STATE is 110 because "Downloading" is 11 cells of
  `Style.font.subtitle` (about 86 px at base size 12) plus 2 × `Style.space(10)` padding; the
  fixed columns without NAME sum to 26 + 110 + 76 + 120 + 80 + 80 + 64 + 56 + 56 + 80 = **748 px**.
  Row height `Style.spacing.popupRowHeight`, header `Style.space(30)`, cell padding
  `Style.space(10)`, numerics right-aligned, name left-aligned with `ElideRight` and the full name
  in the details pane and a tooltip. NAME never goes below `Style.space(210)` (about 26 cells of
  `subtitle`, enough for "The Diplomat S04E01 1080p…"); a tier is valid only if NAME ≥ 210 at the
  tier's narrowest width.
- Double-click on a row is Enter (details); there are no context menus, as in every first-party
  panel. Right-click does nothing in the table (the popup's right-click is the add offer, §4.8).
- Cursor: `Style.selectedAccentFill` on the row and a `Style.space(3)` accent bar at the left
  (`omaqbt TorrentTable.qml:205-217`); multi-select range `Style.selectionFill`; a row named in a
  failed action shows `!` in urgent until the next key (omaqbt `TorrentTable.qml:225-226`).
- The progress cell is a text bar of block characters in the shell font (so it aligns with the
  monospace grid and needs no custom paint) plus the percentage; "done" replaces the percentage
  at 100 %. Unknown shows an empty bar and "—".
- Sorting: click any header (it sorts by that column, ↑, PEERS and RATIO included) or press `s`
  (which cycles the §4.6 list); the sorted header is in foreground with an arrow glyph
  (omaqbt `TorrentTable.qml:69-72`); the sort key and direction persist in the plugin-owned
  `~/.local/state/fbittorrent/dashboard.json` (§4.6; panels receive no `settings`).
- The list is a `ListView` over a `ListModel` patched in place (set/insert/remove/move), with
  `reuseItems: true`, `currentIndex: -1` and the cursor painted by the delegate from a hash, so a
  status tick never resets scroll (omaqbt `TorrentTable.qml:9-15,166-179`).

#### 4.3.3 Column priority by width

The dashboard adapts by window width (`FloatingWindow.width`; `minimumSize` 900×560;
`implicitWidth/Height` 1600×900, omaqbt's size, so a floating first open lands in the full tier;
the owner's tiled half-screen window of about 960 px lands in the chip-row tier, which is designed
for it, not degraded to it). Each tier below is checked: list width = window − chrome (§4.2.2),
NAME = list − fixed columns (§4.3.2); every tier keeps NAME ≥ 210 at its narrowest width, and
column visibility is monotonic (a column hidden at one width is hidden at every narrower width).
Every number here is in `Style.space` units: the tier is chosen from
`width / Style.effectiveSpacingScale` (`Commons/Style.qml:213`, the font scale times the spacing
scale), so a scaled shell keeps the same proportions; `qs-dashboard.sh::width_tiers` runs at
scale 1 and logs the scale it saw.

| Window width | Sidebar | Details | Table columns (in addition to glyph, STATE, NAME, PROGRESS, ↓, which never hide) | Chrome | Fixed columns | List width → NAME |
|---|---|---|---|---|---|---|
| ≥ 1600 | docked (200) | docked (360) | SIZE, ↑, ETA, PEERS, RATIO, ADDED | 636 | 748 | ≥ 964 → ≥ 216 |
| 1400 to 1599 | docked | docked | SIZE, ETA, PEERS (↑, RATIO, ADDED hidden) | 636 | 532 | 764–963 → 232–431 |
| 1100 to 1399 | docked | opens as a sheet over the list (Enter), closes with Esc | SIZE, ETA, PEERS (↑, RATIO, ADDED hidden) | 246 | 532 | 854–1153 → 322–621 |
| 900 to 1099 | collapsed to a chip row above the table ("All 12 · Downloading 2 · …") | sheet | SIZE, ETA, PEERS (↑, RATIO, ADDED hidden) | 16 | 532 | 884–1083 → 352–551 |

Why ↑ goes at 1400–1599 and not only RATIO and ADDED: with ↑ kept the fixed columns are 612 px
and NAME is 152–351 px, below the floor at the tier's narrow end. The two narrow tiers show the
same columns as the 1400 tier and spend their extra width on NAME, which keeps visibility
monotonic (the round-2 review's draft re-showed ↑ at 1100–1399 and hid it again at 1400–1599,
which would make a column reappear while the window shrinks).

Drop order as width shrinks: ADDED → RATIO → ↑ → PEERS → ETA → SIZE; the details pane undocks
(1399) before PEERS would be dropped, because the pane costs 360 + 14 + 16 = 390 px and PEERS 56.
Hidden columns' values remain in the details pane and in the row tooltip. The `hideColumns` list
is computed once per width change, not per row (omaqbt `TorrentTable.qml:36-38`).

#### 4.3.4 State words and glyphs (the shared vocabulary)

`Model.js` and `src/tui/format.rs` carry the same table (one fixture, `contracts/ui/states.json`,
checked by `qs-format.sh::states` and `format::tests::states`): per normalized `state` (§3.5) one
Nerd Font glyph (QML), one text glyph (TUI and CLI, display width 1), one word, and one `short`
word of at most five cells for the TUI's 9-cell state column (§5.5.3). The popup and the table use
glyph + word; the TUI column uses text glyph + `short`; `fbittorrent list` uses the word. The
Shows and RSS transfer symbols are the same rows (`queued` is `…`, as the Shows screen already
shows on help page 3: "↓ downloading · … queued/waiting · ‖ paused · ✓ complete · + added
earlier").

`contracts/ui/states.json` is a JSON array in this table's row order; each element is
`{"state": "downloading", "qml_glyph": "U+F041A", "text_glyph": "↓", "word": "Downloading",
"short": "Down", "secondary": "rate · eta · peers"}`. Glyphs are written as `U+XXXX` codepoints
(P2-04 pins the MDI ones), so the file is font-independent; the `ready`, `disk_full`,
`added_before`, `in_list` and `pending` rows use the same shape with those names in `state` and
the pending verbs as a `verbs` array on the `pending` row.

| `state` | QML glyph | Text glyph | Word | `short` | Secondary (when present) |
|---|---|---|---|---|---|
| `metadata` | dots-horizontal + spinner | `◌` | Metadata | Meta | "0:12" elapsed |
| `checking` | sync | `⟳` | Checking | Check | percent checked |
| `downloading` | `󰇚` | `↓` | Downloading | Down | ↓ rate · ETA · peers |
| `stalled` | timer-sand | `~` | Stalled | Stall | "no peers" or "no data for 2 min" |
| `queued` | clock-outline | `…` | Queued | Queue | "3 of 3 slots busy" |
| `paused` | `󰏤` | `‖` | Paused | Pause | percent |
| `done` | `󰄬` | `✓` | Done | Done | size |
| `seeding` | `󰕒` | `↑` | Seeding | Seed | ↑ rate · ratio |
| `waiting_vpn` | `󰦝` | `⏚` | Waiting for VPN | VPN | interface name |
| `error` | `󰀦` | `!` | Error | Error | the message |
| `error` with `reason: disk_full` (P5-07) | `󰀦` | `!` | Disk full | Disk | "1.2 GiB free, 3.1 GiB needed" |
| ready (P5-05, overlays `downloading`) | play-circle-outline | `▶` | Ready to watch | Ready | "4 min buffered", or "may stall" (§3.5) |
| added earlier (Shows, RSS) | plus | `+` | Added before | n/a | |
| in your list (Search, B10) | plus | `+` | In your list | n/a | |
| unread (Shows, RSS) | `●` (U+25CF, in the shell font) | `●` | Unread | n/a | |
| pending verb | the state's glyph at 0.55 opacity | the state's glyph | "pausing…", "resuming…", "removing…", "adding…", "checking…" | | |

`◌` (U+25CC) replaces `…` for metadata because `…` already means queued/waiting on Shows, and
`⋯` (U+22EF) is dropped because it is indistinguishable from `…` at a glance, which defeats the
glyph-plus-word rule in a 9-cell column; the width test of §5.5.3 and the glyph render test of
P2-04 cover `◌`, and `?` is the fallback if the font lacks it.

The `reason` values of §3.5 render as these phrases (a `reason` row of `contracts/ui/states.json`
carries them; the TUI details strip, the dashboard Overview and `fbittorrent show` use them):
`user` "paused by you"; `queue_full` "waiting for a download slot"; `complete_no_seed` "complete;
seeding is off"; `vpn_down` "waiting for <interface>"; `restore_failed` "could not restore:
<error>"; `metadata_timeout` "no peers answered"; `missing_files` "files are missing";
`disk_full` "disk full". Times render through `Model.when(ts)` and `text.rs::when`: "12:41
today" for today, "Oct 12" for this year, "2025-10-12" otherwise; relative times ("12 s ago")
through `Model.ago(ts)` and `text.rs::ago`.

### 4.4 Service: one connection for the whole plugin

Today each bar widget instance creates its own `Service` and its own processes
(`Panel.qml:119-122`), so polling multiplies by the number of monitors [FO §1.4]. In v0.2 the
`service` kind is a singleton that the bar widgets and the dashboard reach through the host's
`service`/`serviceFor` facade (`services/PluginShellApi.qml:30-32`) [FO §2.2].

Socket client rules, each learned the hard way by Sprout or omaqbt:

1. `Socket { path: <XDG_RUNTIME_DIR>/fbittorrent/api.sock }`, with the runtime directory read
   from the environment (`Quickshell.env("XDG_RUNTIME_DIR")`), overridable by the test hook
   `useRuntimeDir` (§4.17).
2. `write()` adds no newline and buffers: always send `JSON.stringify(frame) + "\n"` and call
   `flush()` after it (`quickshell-src/src/io/socket.hpp:37-43`) [OR §6 gotcha 4].
3. An initial `connected: true` does not fire `connectionStateChanged`; the hello is sent from a
   bootstrap path that checks `connected` after creation as well as from the signal handler
   [OR §6 gotcha 4].
4. There is no automatic reconnect. A `Timer` sets `connected = true` again with backoff 0.5, 1,
   2, 4, 8 s, then every 10 s, independent of socket signals [FO §2.6; OR §8 item 7].
5. A `SplitParser` splits lines. The server's 1 MiB cap (§3.4.1) is the memory bound, because
   `SplitParser` buffers until the delimiter with no cap of its own
   (`quickshell-src/src/io/datastream.cpp:37-41,91-98`); a received line longer than 1 MiB means a
   protocol violation and resets the session. `JSON.parse` is wrapped in `try`.
6. Requests carry increasing ids. The pending map times out after 20 s (100 s for `torrents.add`),
   always longer than the server's own op timeouts (15 s and 95 s, §3.4.10), so an answer can never
   arrive after the client has given up on it; a timeout marks that action "no answer". After a
   reconnect, pending idempotent mutations (A.1) are resent with the same `ref`; nothing else is.
7. 45 s without any frame (heartbeats arrive every 15 s) resets the session.
8. `hello` uses `kind: "bar"` and `interactive: false`. When the dashboard window becomes visible
   the service sends `app.client {interactive: true}`, and `false` when it hides, so the bar
   never keeps the daemon alive on its own (A1). When the popup opens the service sends
   `app.client {glance: true}` and `false` on close, which pauses the idle countdown (§3.2.10)
   without counting as interactive; the daemon treats a connection drop as `glance: false`.
9. Subscriptions follow visibility: `summary` at 5,000 ms when nothing is open; `summary` and
   `torrents` at 1,000 ms while the popup is open or the dashboard is visible; detail topics only
   for the torrent whose details are on screen. After every reconnect the service resubscribes.
10. When the socket cannot connect, `online = false` and the backoff caps at 10 s; `reconnecting`
    is true from an EOF without `bye` until the backoff first reaches that cap (about 15 s), and
    false on the next `hello` or on a `bye`. The service never starts the daemon on its own; only the explicit `startDaemon` action (below) does, and
    only when a person pressed or clicked Start.
11. Revision checks (§3.4.7): a gap resubscribes that topic; a new `instance` resubscribes all.

The Store: `Store.js` reduces frames into plain objects keyed by info hash. `Service.qml` mirrors
them into `ListModel`s, updating changed fields in place with `setProperty` and inserting or
removing rows only on membership changes. Sorting runs in `Store.js` on membership change, on a
sort-key change, and at most once per second for rate-based sorts. Why client-side sorting is
acceptable here: the list has tens to a few hundred rows, so a once-per-second sort is cheap;
P2-02 measures it at 500 rows, and if it exceeds 2 ms the sort moves into the subscription
arguments on the server. `ScriptModel` keyed by `objectProp` is not used, because in Quickshell
0.3.2 it does not emit row updates [FO §2.6].

Actions exposed by the service: `pause`, `resume`, `toggleSeed`, `remove(hash, deleteFiles)`,
`addFromClipboard`, `add(source, stage)`, `commitStaged`, `discardStaged`, `openTarget`,
`setTurtle`, `recheck`, `quit`, `startDaemon`, `dismissClipboard`. Every mutation gets
`ref = "qs-" + startMillis + "-" + n`. `startDaemon` runs `fbittorrent up` once through
`Util.execArgv` and is the only way the plugin ever starts the daemon: the dashboard's [Start],
the popup's Enter on its "not running" state (§4.11) and a click on the dimmed icon under
`showWhenStopped` (§4.5) call it; nothing calls it without a person's action. It is a one-shot,
not a poller.

- **Clipboard:** `Quickshell.clipboardText` is empty unless a Quickshell window has focus
  [FO §2.6], so the service reads it with a short-lived `wl-paste --no-newline` process, accepts a
  magnet, an http(s) URL or a `.torrent` path, and checks the btih against the store first:
  "Already in your list: <name>" instead of a second add (Fragments' dedupe [FO §5]).
- **Opening files:** `torrents.open_target` returns a path; the service runs
  `xdg-open <path>` with `Util.execArgv`, the same thing the CLI's `open` does
  (`src/main.rs:633-641`).

### 4.5 Bar icon and popup

**Icon states.** State is carried by the glyph as well as the colour, so the monochrome theme
works; the tooltip always carries the words.

| State | Glyph | Colour | Tooltip | Rate text (`showRates`) |
|---|---|---|---|---|
| Daemon not running | hidden by default; with `showWhenStopped`, `󰇚` | dim (`Qt.darker(barForeground, 1.4)`) | "fbittorrent is not running · click to start" | none |
| Running, nothing active | `󰇚` (the brand mark; state is carried by weight, not shape: `󰕒` is `nf-md-upload` and reads as "uploading", the one thing seeding-off-by-default guarantees is not happening) | dim | "Idle · 8 done", or "Idle · stops in 4 min" | none |
| Seeding only (no download active) | `󰕒` | `barForeground` | "↑ 120 KB/s · 1 seeding" | "120K" |
| Stopped recently (10 s after `bye`, §4.5 visibility) | `󰇚` | dim | "fbittorrent stopped (idle)" or "fbittorrent stopped" after the power button (from `bye.reason`) | none |
| Downloading | `󰇚` | `barForeground` | "↓ 4.2 MB/s ↑ 32 KB/s · 2 downloading · 1 queued" | "4.2M" |
| Ready to watch (a torrent became playable and has not been acknowledged; acknowledged means the popup or the dashboard was opened after the event, `o` was pressed on it in any UI, or it completed; the daemon keeps the flag as `Torrent.playable.acked` so the TUI and the bar agree) | play-circle-outline | `barForeground` | "Ready to watch: <name>" | as downloading |
| Waiting for the VPN | `󰦝` | `bar.urgent` | "Waiting for ProtonVPN · 2 downloads waiting" | none |
| Engine error, or a torrent in error | `󰀦` | `bar.urgent` | the message | none |
| Turtle on | the state's glyph plus a small tortoise glyph after it | as the state | "… · turtle limits on" | |

Clicks: left toggles the popup; right opens the popup with the quick-add offer strip of §4.8 already showing (today it adds at once, `Panel.qml:149`); middle
opens the dashboard. Today middle opens the TUI (`Panel.qml:150`); `middleClick = "tui"` restores
that, and `t` in the popup always opens the TUI. This is a deliberate default change, logged as
decision DL-12. Colour changes animate for `Style.duration(160)` only when
`bar.foregroundAnimationEnabled` (the Sprout pattern, `Sprout BarWidget.qml:306-320`).

Visibility: `visible: svc.online || showWhenStopped || root.opened || svc.stoppedRecently || svc.reconnecting`
(`reconnecting` is true from an EOF without `bye` until the backoff reaches its 10 s cap, about
15 s, so a crash-restart under `Restart=on-failure` shows the §4.11 "Reconnecting" icon instead
of a vanish-and-return),
with `showWhenStopped` default `false`, which keeps the owner's "visible only while the app runs"
rule. `root.opened` keeps the anchor of an open popup on the bar, so the popup can never close
under the cursor because its icon vanished; `svc.stoppedRecently` is true for 10 s after a `bye`,
during which the icon is dim with the tooltip "fbittorrent stopped (idle)", or "fbittorrent stopped"
after the power button (`bye.reason`), and the popup, if
open, shows the "stopped" state of §4.11 ("fbittorrent stopped. Enter Start · w Open window").
Under A1 the icon therefore disappears a few minutes after the last download, visibly rather than
mid-glance, and the menu, a keybinding or `fbittorrent dashboard` bring the app back. The
reality check lists the hidden widget as defect 15 ("the bar offers no way to open the app"); the
owner answered Q2 on 2026-10-18 with Q1: hidden when stopped, so that a lit icon always means a
running process (A1); the setting stays, off, for anyone else. With `showWhenStopped`, a left click
on the dimmed icon starts the daemon (`startDaemon`) and opens the popup in its "Starting…" state.

**Popup** (`KeyboardPanel`, `Style.space(380)` wide like every first-party popup, height fitted up to
`Style.space(560)`, `Panel.qml:162-163`; content passed as `content: PanelKeyCatcher { … }` so it
is built on first open, per `shell-dev.md` "Plugin contract"; drawn in keyboard mode, after a key
press, so the KeyHint row is present; at rest the popup ends with the collapsed-groups row and has
the Wi-Fi panel's structure, §4.2.3):

```
┌────────────────────────────────────────────────────┐
│ 󰇚  Torrents                      󰐕  ▢  󰢃  󰐥        │  PanelHero: add · window · turtle · quit (trailingControl)
│    ↓ 4.2 MB/s  ↑ 32 KB/s        2 DOWNLOADING      │  meta (uppercase caption); detail pill
│ ────────────────────────────────────────────────── │  PanelSeparator
│ ▲ Waiting for ProtonVPN (proton0). Downloads       │  Banner, only when relevant (urgent tint)
│   resume when it connects.                         │
│ ────────────────────────────────────────────────── │
│ DOWNLOADING                                        │  PanelSectionHeader
│ 󰇚 The Diplomat S04E01 1080p WEB…                   │  PopupRow (§4.3.1)
│   ▕██████████▌                  ▏                  │
│   Downloading · 43% · ↓ 3.1 MB/s · 12 min · 41 pe… │
│ ▶ Some.Movie.2026.1080p…                           │
│   ▕████▌                        ▏                  │
│   Ready to watch · 18% · 4 min buffered   [Open]   │  P5-05: accent badge; Enter/o opens
│ QUEUED 1 · PAUSED 1 · DONE 8                  ›    │  collapsed groups; Enter expands in place
│ ────────────────────────────────────────────────── │
│ a add · / search · w window · t terminal · z turtle│  KeyHint row (caption; present only after a key press, §4.2.3)
└────────────────────────────────────────────────────┘
```

- Rows show the glyph, the name (middle elision, full text on hover as today), the state word,
  percentage, rate, ETA and peers when known. Hover or the cursor reveals row actions: pause
  (`p`), open (`o`), remove (`d`).
- Collapsed groups expand in place (at most 10 rows each); `w` opens the dashboard on that filter.
- **Remove** opens an inline confirm strip in the row: "Remove '<name>'? Files are kept." with
  [Cancel] [Remove]; focus starts on Cancel, `y` or a click on Remove confirms. The popup never
  offers "delete files"; that lives in the dashboard and the TUI. Why: a glance surface should not
  hold the one irreversible action.
- **Stop** (the power icon `󰐥`, last of the hero's trailing controls, or `Q`; `q` closes the popup,
  as it closes the TUI and the dashboard): the owner's off switch (A1). With nothing active it sends
  `app.shutdown {reason: "user"}` at once; with transfers active it asks in an inline strip under
  the hero, "Stop fbittorrent? 2 downloads pause until it starts again." [Cancel] [Stop], Cancel
  focused, `y` or Enter on Stop confirms, Esc cancels. The daemon drains, the icon turns dim for the
  10 s "stopped recently" state and then leaves the bar (§4.11): the bar shows exactly what runs.
- Banners appear only when relevant (§4.11). The "stops in 4 min" idle countdown is a dim line
  under the hero, not a banner.
- The popup's `IpcHandler` keeps `open/close/toggle/refresh/addClipboard/tui` and the `cursor()`
  test hook (`Panel.qml:124-139`), and adds `useRuntimeDir(path)` and `state()` test hooks (§4.17).

### 4.6 Dashboard window

- The window is an application window, not a popup, and reads as one: numbered tabs, a key row, a
  sidebar with counts, none of which a first-party panel has. It is Omarchy-native through what it
  is built from, not through popup structure: `Commons.Color.background`,
  `Border.controlSpec("normal")` frames, `FilterInput`, `ConfirmDialog`, `ToggleSwitch`,
  `Dropdown`, `Style` spacing and the monospace family.
- `Dashboard.qml` is the `panel` entry point. `open(payloadJson)` creates or shows a
  `FloatingWindow` titled `fbittorrent` (`minimumSize` 900×560, `implicitWidth/Height` 1600×900;
  the properties exist in the installed Quickshell:
  `/usr/lib/qt6/qml/Quickshell/_Window/quickshell-window.qmltypes:47-104`). The payload selects
  the view: `{"view": "torrents"|"search"|"shows"|"rss"|"settings"|"log", "query": "…",
  "hash": "…", "filter": "downloading"}`. `close()` hides it; user-initiated closes go through
  a separate `dismiss()` that calls `shell.hide("oliver.torrents")`, because calling `shell.hide()`
  from inside `close()` recurses [OR §6 gotcha 4]; the dev gallery does the same
  (`plugins/dev-gallery/GalleryPanel.qml:286-289`).
- Surfaces: the window paints `Commons.Color.background` with text in `Commons.Color.foreground`
  (the dev gallery's choice, the only first-party `FloatingWindow`: `GalleryPanel.qml:71,281`);
  pane frames use `Border.controlSpec("normal")`; the add sheet, the details sheet and the help
  overlay are cards on a `Util.alpha(Commons.Color.background, 0.7)` scrim with
  `Border.surfaceSpec("popups", "border", …)`, the same recipe as `Ui/ConfirmDialog.qml:13-15,52-58`;
  tooltips use `PanelToolTip`. `Color.popups.background` is never used for the window itself: on
  themes with a translucent popup alpha it would render a see-through application window.
- Window identity: every Quickshell window has class `org.quickshell`, so Hyprland rules match the
  title `^fbittorrent$` [FO §2.3]. By default no rule is installed and the window tiles like any
  application; the README documents an optional float rule.
- Focus: a programmatic open does not reliably focus the window. After opening, the dashboard asks
  for activation in the order omaqbt found necessary: `forceActiveFocus`, `requestActivate` on
  the backing window, the foreign-toplevel `activate`, then a Hyprland focus dispatch by address,
  giving up after 1.5 s with a log line (`omaqbt WmFocus.qml:8-17,85-130`). P3-08 verifies that a
  keybinding summon lands keyboard focus in the window.
- When the daemon is not running the window shows an empty state, "fbittorrent is not running",
  with [Start] focused (Enter): `startDaemon()`, then the service reconnects.

```
┌ fbittorrent ─────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ 1 Torrents  2 Search  3 Shows ●4  4 RSS ●2   [/ filter…          ]  ↓ 4.2M ↑ 32K  󰢃  󰦝 proton0                          │
├──────────────┬────────────────────────────────────────────────────────────────────────────────┬──────────────────────────┤
│ All        12│ STATE        NAME                        SIZE  PROGRESS      ↓    ETA  PEERS   │ ┌──────┐ The Diplomat    │
│ Downloading 2│ 󰇚 Downloading The Diplomat S04E01 1080p…  1.1G ▕███▌  ▏43%  3.1M  12m     41   │ │poster│ 2023–           │
│ Ready      1 │ ▶ Ready       Some.Movie.2026.1080p       2.1G ▕█▌    ▏18%  2.0M  18m     12   │ └──────┘ ★ 7.9           │
│ Queued      1│ … Metadata    Other.Movie.2026.1080p           ▕      ▏ —      —    —      3   │ Political thriller…      │
│ Paused      1│ 󰏤 Paused      Linux.ISO.x86_64            1.2G ▕██    ▏18%     —    —      —   │ i IMDb   w Wikipedia     │
│ Done        8│ 󰄬 Done        Sintel.2010.1080p           1.4G ▕██████▏done    —    —      —   │ Overview Files Peers     │
│ Seeding     0│                                                                                │ Trackers Pieces Log      │
│ Errors     0 │                                                                                │ ☑ S04E01.mkv 1.1G 43%    │
│ ──────────── │                                                                                │ ☐ sample.mkv  40M        │
│ From: Search 4 RSS 2 Shows 3 Manual 3 (P6-08)                                                 │                          │
│ Shows     ●4 │                                                                                │                          │
│ RSS       ●2 │                                                                                │                          │
├──────────────┴────────────────────────────────────────────────────────────────────────────────┴──────────────────────────┤
│ Enter details  p pause  u seed  d remove  D delete files  a add  o open  R recheck  z turtle  ? keys                     │
│ Added "Some.Movie.2026.1080p" · 12:41                                        ↓ ▁▂▅▇▆▃▂                                   │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

(The mockup is about 120 text columns wide and shows the ≥ 1600 px tier; at the owner's tiled
width of about 960 px the window is in the chip-row tier of Appendix C.)

- **Top bar:** view tabs `1`-`4` with unread badges (`Color.accent` dot plus count), the filter or
  search field (`FilterInput`, `Ui/FilterInput.qml`), rates, the turtle indicator (P5-04), and the
  VPN pill when binding is configured.
- **Sidebar:** state filters with live counts, always in the same order so rows never move under
  the `j/k` cursor or the mouse: All; Downloading (downloading, stalled, metadata, checking);
  Ready (P5-05); Queued; Paused; Done; Seeding; Errors; Waiting for VPN (present only while
  binding is configured, which changes only in Settings). A filter with a zero count is dim and
  still selectable (Transmission's filter bar keeps all entries and greys the empty ones). Later
  "From: Search, RSS, Shows, Manual" (P6-08). Then links to Shows
  and RSS with unread counts. The active filter uses `selectedFill`; the cursor hover-cursor fill.
  Transmission's filter set (All, Active, Downloading, Seeding, Paused, Finished, Verifying, Error;
  `Transmission gtk/FilterBar.cc:474-482`) is the model, with "Ready" added for this product.
- **List:** §4.3.2 and §4.3.3. Sort with `s` (Added, newest first, which is today's order; then
  Name, Progress, ↓, Size, State, ETA); `S` reverses; the sort key and direction persist in
  `dashboard.json` (above).
- **Details pane** on the right (`Style.space(360)`; resizable by dragging the pane edge between
  `Style.space(300)` and `Style.space(480)` and never past the point where NAME would drop below
  210 px; the width persists in `~/.local/state/fbittorrent/dashboard.json`, a plugin-owned file
  written by `Service.qml` through `Quickshell.Io.FileView` with a 500 ms debounce, because panels
  receive no `settings` from the shell). When the window is narrower than 1,400 px, details open as
  a sheet over the list instead (§4.3.3: with the pane docked a 1,399 px window cannot keep NAME at
  210 px).
- **Bottom rows:** contextual keys for the focused pane (`KeyHint` row), and the last message with
  its time (clicking it opens the Log view), plus the speed sparkline (P6-03).
- **Pane focus:** the focused pane's title is in foreground with a `Style.space(2)` accent
  underline; others are dim. `h`/`l` move focus; a click focuses.

### 4.7 Details pane

| Tab | Content | Topic | Item |
|---|---|---|---|
| Overview | Poster and IMDb card (title, year, rating, blurb, `i` IMDb, `w` Wikipedia); state and reason; "Ready to watch · 4 min buffered" with [Open] (P5-05); size, progress, downloaded, uploaded, ratio, ETA; save path (copy), content path (open); info hash (copy); added and completed times; seed switch (`u`, `ToggleSwitch`) with its meaning spelled out ("upload capped at 32 KiB/s while downloading; stops when complete", "seeding allowed", or "upload capped at N KiB/s" for a custom cap); queue position; an action row of `Button`s ("Recheck…" P5-03, "Move…" P7-02), each hidden when its cap is absent | `torrents` + `torrents.get` once + `imdb.lookup` | P3-10 |
| Files | Natural sort (S01E01 before S01E07), include/skip checkboxes (Space), all/none (`a`/`n`), per-file progress, `o` opens the file, filter; a per-file "ready" dot (P5-05) | `torrent:<h>.files` | P3-10 |
| Peers | Address, client, connection kind, downloaded, uploaded, rates, seed flag | `torrent:<h>.peers` | P5-02 |
| Trackers | URL, status, last announce, peers returned, error; reannounce (`e`) | `torrent:<h>.trackers` | P6-01 |
| Pieces | Pieces bar from the have-bitfield; first and last pieces marked; the playable prefix shaded | `torrent:<h>.pieces` | P6-02 |
| Log | Log entries for this torrent | `log` (filtered) | P3-10 |

The tab set follows qBittorrent's property tabs (General, Trackers, Peers, Content, Speed;
`qBittorrent src/gui/properties/proptabbar.cpp:50-100`) minus HTTP sources (§2.6) plus Pieces
and Log; the "Speed" tab's graph becomes the status-line sparkline (P6-03). Why the two
additions: Pieces is the one view that shows sequential plus first/last working, which is what
a streaming user doubts first when a file will not play (§6.2 row 25); Log answers "why is this
torrent not moving" next to the torrent, where today the message disappears after 8 or 15 s
([RC §9] #13).

### 4.8 Add flow

Sources:

- the popup's `a` and the right click: the offer strip of P6-13 with the magnet's `dn=` name (or
  the `.torrent` file name, or the URL's last path segment), "Add '<name>'? [Add] [Cancel]" with
  Add focused; Enter adds, Esc cancels; a duplicate shows "Already in your list: <name>" instead
  of the strip, and an empty or unrecognised clipboard shows "Clipboard has no torrent" for 3 s.
  One Enter more than today, in exchange for seeing what is about to be downloaded (principle 2),
  one component shared with P6-13, and a right-click that cannot add the wrong thing. The handler
  and the CLI stay immediate: there the click or the command is the deliberate act;
- the dashboard's `a` or `Ctrl+V`: the add sheet, prefilled from the clipboard;
- drag and drop onto the window (`DropArea`: file URLs become paths, dropped text is checked for a
  magnet); unverified on Hyprland, P3-11 tests it;
- the `magnet:` handler and the CLI: immediate, with a notification (A5); `handler = "confirm"`
  opens the sheet instead.

The add sheet:

1. `torrents.add {source, stage: true}` creates a staged, paused record that the queue and the
   counts ignore. A magnet gives the hash at once; URLs and files give it with the metadata.
2. The sheet shows the name, the size once known (otherwise "resolving metadata… 0:12"), the file
   list with checkboxes in natural order, the seed switch (default `seed_by_default`; the
   equivalent of today's F4 toggle, `README.md:67-69`), and the destination (default
   `~/Downloads`; P6-11 adds a picker with free space). A duplicate answers "Already in your list"
   with the existing torrent.
3. Enter sends `torrents.commit {hash, files, seed}` and the torrent joins the queue. Esc sends
   `torrents.discard {hash}`, which removes the staged record and **never deletes files**: a
   re-added torrent may point at data that already exists in `~/Downloads`.
4. If metadata does not arrive within 90 s: "No peers answered yet." [Keep paused] [Discard].

Why a staged add in the dashboard: it matches "deliberately download" and omaqbt's add-stopped,
then-confirm flow [FO §8.3]. Quick add stays for the popup, the CLI and the magnet handler, where
the click itself is the deliberate act.

### 4.9 Destructive actions

- Remove is always a choice between two labelled actions: "Remove (keep files)" and "Remove and
  delete files". The dashboard uses Omarchy's `ConfirmDialog`
  (`/usr/share/omarchy/shell/Ui/ConfirmDialog.qml`) with the name, the size and the path; "delete
  files" also shows the total bytes and uses the urgent colour (the dialog's own index-1 button
  is styled destructive, `ConfirmDialog.qml:97-106`).
- **The shell's dialog defaults focus to the destructive button** (`property int selectedIndex: 1`,
  `ConfirmDialog.qml:12`). Every fbittorrent confirm sets `selectedIndex: 0` on open, and a qs
  fixture test asserts that Enter immediately after opening cancels.
- `d`, `x` and Delete open "Remove (keep files)"; `D` opens "Remove and delete files". Enter
  activates the focused button (Cancel by default); `y` confirms. The shell's dialog handles only
  Esc, arrows, Tab and Enter (`Ui/ConfirmDialog.qml:24-40`), so the dashboard's key handler maps
  `y` to `confirmed()` before delegating to `handleKey`. Hovering a button selects it
  (`onEntered: root.selectedIndex = index`, `Ui/ConfirmDialog.qml:122`), so "focus starts on
  Cancel" holds only until the pointer moves; the P3-09 fixture test moves the pointer off the
  dialog before pressing Enter.
- Bulk removal of a filtered set (P6-08) shows the count and total size and requires `y`.
- The popup offers only "Remove (keep files)" (§4.5).
- The API makes `delete_files` mandatory (§3.4.8). Unfollowing a show and removing a feed also
  confirm, with Cancel focused.
- A keep-files removal shows an undo toast for 10 s in the dashboard and the TUI (P6-08).

### 4.10 Pending and confirmed

A row with a pending mutation shows the verb in progress ("pausing…", "removing…") in dimmed
italics with a small spinner glyph (static "…" under `Style.reduceMotion`), and keeps its last
confirmed values. The delta or `mutation.result` clears it. A failure restores the row, shows a
banner and writes the log. A timeout says "Result unknown; the list will tell" and offers
"Refresh" (resubscribe). Nothing is drawn optimistically.

### 4.11 Empty, offline, degraded and error states

| Condition | Bar icon | Popup | Dashboard |
|---|---|---|---|
| Daemon not running | hidden (dimmed with `showWhenStopped`) | (if shown) "fbittorrent is not running" with "Enter Start · w Open window" | empty state with [Start] |
| Starting (daemon up, slot `Starting`) | idle glyph | "Starting… checking 2 torrents" | rows show `checking`; status line "starting" |
| Connecting (first 2 s) | last known | "Connecting…" | "Connecting…" |
| Reconnecting (after 2 s) | last known, dim | "Reconnecting… (retry in 4 s)" with last-known rows dimmed | same; rows dimmed and labelled "last known" |
| Protocol major mismatch | error glyph | urgent banner: "This plugin speaks API 1; the running fbittorrent speaks API 2. Update the plugin or run `fbittorrent restart`." | same |
| Version skew, same protocol | normal | dim banner: "fbittorrent 0.2.0 runs; 0.2.1 is installed. Restart to update." | same |
| Parked for the VPN | urgent | urgent banner: "Waiting for ProtonVPN (proton0). Downloads resume when it connects."; rows dimmed, progress absent | same; sidebar shows "Waiting for VPN" |
| Engine failed to start | urgent | banner with the message and "Show log" | same |
| No torrents | idle | "Nothing here yet. Copy a magnet link and press a, or press / to search." | same, with the search field hint and the key list (omaqbt's state copy pattern, `TorrentTable.qml:285-341`) |
| Filter matches nothing | n/a | n/a | "No torrents match this filter." |
| Mutation result unknown | n/a | "Result unknown; the list will tell." | same |
| Upgrade pending | normal | dim: "Update installed; restarts when idle" | status line |
| Idle countdown | dim | dim: "Idle · stops in 4 min" | status line |
| Clipboard magnet (P6-13) | normal | strip: "Add 'Sintel.2010.1080p'?" [Add] [Dismiss] | banner with the same actions |
| Quick add offer (`a` or right click, §4.8) | normal | strip: "Add 'Sintel.2010.1080p'? [Add] [Cancel]" (Add focused); "Already in your list: <name>"; "Clipboard has no torrent" | n/a (the dashboard opens the add sheet) |
| Stopped recently (10 s after `bye`) | dim, kept visible | "fbittorrent stopped. Enter Start · w Open window" | empty state with [Start]; rows dimmed "last known" for the 10 s |
| Playable, may stall (P5-05) | as downloading | row badge "Playable · may stall (finishes in 18 min)"; `o` still works | same badge; details line with the rate needed |
| Disk low (P5-07) | urgent | urgent banner: "Not enough space for the running downloads: 3.1 GiB needed, 1.2 GiB free" | same; the add sheet refuses a torrent larger than the free space |
| Disk full (P5-07, a torrent hit `ENOSPC`) | urgent | row `Disk full`; the banner above | same; sidebar "Errors" counts it |

Layout rule from Sprout's longest-lived bug: empty and degraded messages must never be children of
a `ListView` whose height collapses to 0 with no rows; they sit in a fixed area as overlays, and
the area has a minimum height only when the count is zero [OR §6 gotcha 1].

### 4.12 Keyboard model

- `PanelKeyCatcher` turns keys into signals: `j`/`k`/`h`/`l` and arrows move, Enter and Space
  activate, `x`, `X` and Delete request deletion (one `deleteRequested()`, no modifier, so the
  dashboard's bulk `X` is a window `Shortcut` that fires before the catcher, Appendix B), Esc
  closes, Tab and Shift+Tab emit `tabRequested(±1)`, Ctrl+j/k reorder when `reorderable`, any
  other character arrives as `textKey(text, modifiers)`
  (`Ui/PanelKeyCatcher.qml:36-99`). It takes keys before items, so text fields set `blocked`
  while focused; `FilterInput` keeps IME composition working (`Ui/FilterInput.qml:3-6`).
- Esc steps back one layer at a time: dialog, then filter text, then details, then the selection;
  at the top level it does nothing, and `q` closes the window (Appendix B, DL-34: a long-lived
  window must not vanish on a reflex key). This is ucmz851's layered Esc [OR §5] without its last
  step.
- In the dashboard, Tab and Shift+Tab cycle the four views like the TUI; `h`/`l` move focus between
  sidebar, list and details. PgUp/PgDn/Home/End are handled by a `FocusScope` with
  `Keys.priority: Keys.AfterItem` so they bubble past the catcher (the dev gallery pattern,
  `GalleryPanel.qml:303-320`).
- Space pauses and resumes on every torrent list in every surface (on Shows and RSS lists it
  toggles read, Appendix B). `PanelKeyCatcher` emits `returnRequested` and then
  `activateRequested` for Enter, and only `activateRequested` for Space
  (`Ui/PanelKeyCatcher.qml:86-92`); the popup and the dashboard set a flag in `onReturnRequested`
  and treat an `activateRequested` without it as Space. Today's popup already tries to map Space
  to pause (`Panel.qml:180`, `t === " "`) and silently fails because the catcher consumes Space
  first; this is how that bug is fixed instead of documented. The exceptions to "same letters
  everywhere" are listed once, at the end of Appendix B.
- The full keymap is Appendix B; `KeyHint` rows render it from `keymap.json` (generated by P4-02).

### 4.13 Motion

All motion goes through `Style.duration(ms)` so `omarchy toggle animations` turns it off at once
(`Style.qml:360-367`). No surface animates layout.

| Element | Motion | Duration |
|---|---|---|
| Progress rule and table bar fill | width `NumberAnimation`, `Easing.OutCubic` | `Style.duration(400)` (today's literal 400 at `Panel.qml:410` becomes a token) |
| Bar icon colour | `ColorAnimation`, gated on `bar.foregroundAnimationEnabled` | `Style.duration(160)` |
| Banner show/hide | opacity | `Style.duration(140)` (the `KeyboardPanel` fade) |
| Pending spinner | text frames at 8 fps; static "…" when `Style.reduceMotion` | n/a |
| Cursor and hover fills | none (instant, as every first-party panel) | 0 |
| Row insert/remove, sort | none (`ListView` transitions off) | 0 |
| Sparkline | redraw on data, no interpolation | 0 |
| Ready-to-watch badge appearing | none; the notification carries the moment | 0 |
| Window open/close | left to Hyprland | n/a |

### 4.14 Theming

- Colours come only from `qs.Commons` (`Color.*`, `Color.popups.*`, `Style.*`, `Border.*`) and the
  bar's `foreground`/`urgent`; tints derive from tokens (`Util.alpha`). The palette singleton is
  imported as `qs.Commons as Commons` and referenced as `Commons.Color`, because Qt 6.12's
  `QtQuick.Color` shadows the bare name (`shell-dev.md` "Palette references"; omaqbt broke on this
  [OR §1.4]).
- A lint test (P2-05) fails on any literal colour, size or duration in plugin QML and JS (§4.2.1).
- Every release checks captures in the owner's monochrome theme and in one light theme
  (`catppuccin-latte` or `flexoki-light`, both installed under `/usr/share/omarchy/themes/`);
  state must stay legible without hue (§2.2 principle 4). The contrast test of §4.2.1 runs over
  every installed theme in CI-less mode (it needs only the TOML files).
- The brand is the monochrome Nerd Font glyph already used (`󰇚`/`󰕒`); no brand colour.

### 4.15 Hot reload, crashes and the shared UI thread

- Omarchy reloads every plugin when anything under `~/.config/omarchy/plugins` changes, and this
  plugin is a symlink into the repository [FO §2.7]. All state lives in the daemon; the service
  reconnects within a second and resubscribes. An open dashboard window closes on reload; that is
  accepted.
- After a hot rescan the first summon can run a stale component; restart the shell with
  `omarchy-restart-shell` before debugging QML [OR §6 gotcha 4].
- The shell's UI thread is shared with the bar, notifications and the lock screen [FO §2.7].
  Rules: no JavaScript loop over all torrents per delta; `ListView` with `reuseItems`;
  asynchronous image loading from local poster files only (no network from QML); at most 500
  rendered search rows (the daemon caps results at 5,000, `src/search.rs:26`); the poster grid
  (P3-17) renders at most 60 posters and loads them with `asynchronous: true` and `sourceSize`
  set to the cell size.
- Only short-lived processes: `wl-paste`, `xdg-open`, `hyprctl`, `fbittorrent up`. A `Process`
  destructor SIGKILLs its child on reload [FO §2.6], so nothing long-lived may be a child of the
  plugin.

### 4.16 Testing the plugin

- **Store fixtures** (P2-02): `qs -p plugin/tests/store` loads `Store.js` and the
  `contracts/ipc/v1` files, applies every `s2c` frame, compares with each `expect` model, prints
  one `FBTQS-STEP {json}` line per frame (the JSONL log of §9.1) and `FBTQS-PASS <name>` or
  `FBTQS-FAIL <name> <diff>` and exits.
- **Smoke against a disposable daemon** (P2-06): `plugin/tests/qs-smoke.sh` creates temporary XDG
  directories, starts `fbittorrent serve --idle-exit-secs 600 --no-notify` there, and runs
  `timeout 60 qs -p plugin/tests/smoke`. The smoke shell instantiates
  `Service.qml` with the temporary runtime directory, adds
  `magnet:?xt=urn:btih:<40 random hex>&dn=fbt-smoke` (a hash that can never resolve), waits for it
  in the model, pauses and resumes it, removes it with `delete_files: false`, and checks each step
  by marker lines. It never touches the user's daemon because the runtime directory differs
  (Sprout's `tools/test_quickshell.sh` pattern [OR §8 item 9]).
- **Visual captures** (P2-08): `plugin/tests/qs-visual.sh` (opt-in, `FBT_TEST_VISUAL=1`) runs against
  the live shell: it starts a fixture daemon (`daemon.py --serve-fixture <state>`) in a
  temporary runtime directory, points the plugin at it with the `useRuntimeDir` IPC hook, summons
  the popup or the dashboard, captures the output with `grim` (installed), stores the PNG under
  `target/test-logs/visual/<date>/<state>-<theme>.png` (archived with the receipts of §8.3, never
  committed), and restores the hook. With
  `--light <theme>` it switches the theme with `omarchy-theme-set` for the capture and switches
  back. The list of states to capture is `plugin/tests/VISUAL.md` (every row of §4.11 plus each popup
  row state of §4.3.1). Captures are reviewed by a person; the script additionally reports a
  pixel-diff percentage against the previous capture of the same state so an unintended change
  stands out. `--compare <plugin>` (for example `omarchy.network`) captures the named first-party
  panel in the same theme right after ours and writes the two PNGs side by side into one image,
  with our hero title baseline, meta baseline, first `PanelSeparator` y and row pitch (read through
  `state()`) printed under it. A person judges the pair; there is no pixel gate on someone else's
  panel, because anti-aliasing and theme alpha make a 1 px scan flaky and a failing scan would not
  say what to change. "Looks first-party" is the owner's verdict on that image, recorded in the
  receipt.
- **Visual components:** the popup and dashboard import `qs.Ui` and `qs.Commons` from
  `/usr/share/omarchy/shell`; `Service.qml` itself needs `qs.Commons` for `Util.execArgv`.
  Quickshell resolves `qs.*` from the config root, so an isolated `qs -p <dir>` sees them through
  symlinks `<dir>/Commons` and `<dir>/Ui` into `/usr/share/omarchy/shell` (a `QML_IMPORT_PATH`
  cannot work: it would look for `qs/Commons/qmldir`). Unverified until P2-06's `probe-ui.qml`
  runs; the fallback for the visual components is the captures in the real shell.
- **Keyboard:** `widget_keys.sh` (wtype plus the `cursor()` IPC hook, `Panel.qml:134-138`) is
  updated for the new popup and gains cases for the confirm strip and the dashboard
  (`omarchy-shell shell summon` then `wtype`).
- Omarchy's guidance says not to start extra Quickshell instances for plugins
  [FO §2.10]; that is about production. Tests use a separate instance briefly, as Sprout does.

### 4.17 Test hooks (IPC methods on `oliver.torrents`)

| Method | Purpose | Guard |
|---|---|---|
| `cursor()` | where the keyboard cursor is (exists today, `Panel.qml:136-138`) | none |
| `state()` | JSON: `{online, instance, subs: {<topic>: <last rev>}, pending: [<ref>], view}` (P2-01); `view` is `null` in the bar and the dashboard's per-view object otherwise (`{window: {visible, width, height, view}, cursor, selection, filter}` from P3-08; `columns`, `sort`, `toast` from P3-09; the Shows fields of P3-13) | none |
| `useRuntimeDir(path)` | point the service at another `XDG_RUNTIME_DIR`-style root (a fixture daemon's `<path>/fbittorrent/api.sock`) until `useRuntimeDir("")` | refused unless `FBT_TEST_HOOKS=1` in the shell's environment |
| `summonView(json)` | the dashboard's `open(payload)` from the CLI without going through the shell's summon | same |
| `setPaneWidth(px)` | sets the details pane width as a drag would (the harness has no pointer drag), P3-10 | same |
| `scroll(dy)` | scrolls the focused list by `dy` device pixels as a wheel tick would (the harness has no wheel; `wtype` sends keys only), P3-09 | same |

### 4.18 What is borrowed, from where, and why

| Element | Borrowed from | Why |
|---|---|---|
| Table anatomy: column widths in `Style.space`, uppercase letter-spaced headers, cursor bar + fill, hash-keyed cursor, in-place `ListModel` patching, one dim colour for unknown values (omaqbt's `Color.muted` itself is not adopted, §4.2.1) | omaqbt `TorrentTable.qml:45-67,107-135,166-217` | The only Omarchy torrent table that was built on the shell's tokens and reviewed by users |
| Hover-only row actions, pending verb in the state slot, thin progress track, overlay empty/degraded states | Sprout `Panel.qml:1020-1130`; [OR §6 gotcha 1] | Density without buttons; the "cosmetic pending" rule |
| "95 % Omarchy / 5 % brand": monochrome glyph, no brand colour, animation gated on the bar flag | Sprout docs/BRAND.md [OR §5]; `Sprout BarWidget.qml:306-320` | Looks first-party |
| Stop reason as an enum shown in the UI; polling slows when the window is inactive | Fragments `connection_manager.rs:50-55`; [FO §5] | Truthful "why is it not running"; our subscriptions follow visibility |
| Row description text by state ("x of y · rate" / "x of y" / "uploaded · rate") | Fragments `torrent_row.rs:260-290` | The meta line's grammar |
| ETA text rules ("Paused", "Queued", "Checking…" in the ETA slot when there is no ETA) | Fragments `utils.rs:127-139` | Never a number where there is no estimate |
| Filter sidebar with live counts | Transmission `gtk/FilterBar.cc:474-482` | Familiar and complete |
| Notification actions "Open File" / "Open Folder" | Transmission `gtk/Notify.cc:234-253` | Proven wording |
| Details tabs | qBittorrent `proptabbar.cpp:50-100` | The owner's reference app |
| Pieces bar | qBittorrent `properties/downloadedpiecesbar.cpp` | Proves sequential download visibly |
| Focus activation ladder for a FloatingWindow | omaqbt `WmFocus.qml` | The only known working recipe on Hyprland |
| Layered Esc; search field focused on open; copy-magnet and open-folder one-key | ucmz851 [OR §5] | Small, right |

---
## 5. The TUI as a pure client

### 5.1 What stays

Four screens with `1`-`4` and Tab; hover preview, click to select, double-click to activate,
wheel; the two-row footer; `?` help; IMDb cards with kitty posters; F6 settings; the add prompt
with F4; the files modal keys (Space, `1`, `a`, `n`); the Shows and RSS read models (`m`,
`Shift+M`, `u`); every key letter the README documents. These carry the owner's muscle memory and
the 21 interaction tests.

### 5.2 Becoming a client

**Step 1, P1-15 (on the compat socket).** `fbittorrent tui` calls `ensure_daemon()` and then runs
the TUI against the daemon through `engine.sock`, with `qbt::Client`'s native path unchanged and
the 2 s poll kept. Changes:

- The in-process lifecycle checks (`src/tui/mod.rs:1699-1702`) become "daemon gone" detection:
  two failed polls show "fbittorrent stopped" with Enter to start it again.
- `q` and Ctrl+C close the TUI only. `Q` sends `app.shutdown`, after "Stop fbittorrent? 2
  downloads pause until it starts again. y/N" when transfers are active (fixes D9). `q` never
  asks: closing the TUI never stops a transfer (A1).
- The first `q` ever shows a one-time notice, stored in `~/.local/state/fbittorrent/notices.json`:
  "Downloads continue in the background; fbittorrent stops itself when idle. Q here, or the bar
  icon's power button, stops it now."
- `fbittorrent tui` always runs a client in the current terminal, so a second one over SSH
  attaches (today a second launch only focuses the first window, `src/main.rs:787`). The desktop
  entry and menu keep `omarchy-launch-or-focus-tui`, which focuses an existing window by app id.
- The TUI also opens a presence connection on `api.sock` (`hello` with `kind: "tui"` and
  `interactive: true`), so the idle net counts it as interactive, and sends `app.shutdown` through
  it for `Q`;
  its data still flows over `engine.sock` until step 2.
- "On open" RSS and Shows checks stay in the TUI until P3-07 moves them into the daemon.

**Step 2, P4-03 (API v1).** `qbt::Client` is replaced by `api::Client`. A subscription thread
turns API events into `Msg::Api(Event)` through an ftui `Subscription`, the pattern the existing
`SearchSub` uses (`src/tui/mod.rs:309-350`). The 2 s poll disappears. Search, RSS, Shows, IMDb and
browse calls go to the daemon. The TUI keeps a mirror store in `src/tui/store.rs` with the same
reducer semantics as `Store.js`, tested against the same fixtures.

### 5.3 Module split

Target tree for `src/tui/` (today one 3,504-line `mod.rs`):

```
src/tui/mod.rs              run(); Model glue: init, update, view dispatch (< 400 lines)
src/tui/app.rs              TuiApp state, grouped into one sub-struct per screen and modal
src/tui/actions.rs          the action table (created empty in P4-01, filled by P4-02)
src/tui/store.rs            mirror of API state (P4-03)
src/tui/layout.rs           breakpoints; rectangles recorded during render
src/tui/mouse.rs            hit testing on the recorded rectangles
src/tui/footer.rs           the two-row footer, generated from the action table
src/tui/help.rs             help text, generated from the action table
src/tui/text.rs             cell, short, clean, dw, readable_cell, ago, human_count
src/tui/format.rs           sizes, rates, ETA, percent: the Rust twin of Model.js (§4.2.3), shared with the CLI
src/tui/palette.rs          colour roles from Theme, contrast and low-chroma rules (§5.5)
src/tui/screens/{torrents,search,shows,rss}.rs     view and keys per screen
src/tui/modals/{files,providers,settings,prompt,peers,log,details}.rs
src/tui/theme.rs, src/tui/poster.rs                unchanged files
src/tui/tests/*.rs          the 21 interaction tests, moved unchanged; golden renders
```

Method: an isomorphic split first (P4-01). Code moves without behaviour changes; the public
surface stays; `cargo test`, the golden renders at 80×24, 120×30 and 160×45, and the tmux smoke
must be identical before and after. Behaviour changes come in later items, each with its own
tests. Why this order: a split mixed with behaviour changes cannot be reviewed, and the seams the
reality check names (screens, modals, footer, mouse) are already visible in the function list
[RC §10].

### 5.4 One action table

```rust
struct Action {
    id: &'static str,                 // "torrents.remove"
    keys: &'static [KeyChord],        // primary first, then aliases
    scope: Scope,                     // Global | Screen(..) | Pane(..) | Modal(..)
    label: fn(&TuiApp) -> Option<String>,   // None hides the action in this state
    enabled: fn(&TuiApp) -> bool,
    priority: u8,                     // footer packing order
    run: fn(&mut TuiApp) -> Cmd<Msg>,
}
```

- Footer row 1 shows the enabled actions of the focused scope in priority order, keeps `(?) Help`
  reserved at the right, and shows `…` when some did not fit (fixes [RC §9] #11).
- Help lists every action grouped by scope.
- Key dispatch looks up the key in scope order: modal, pane, screen, global.
- Labels can depend on state, so a finished torrent shows `(U) Seed` and never `(P) Resume`
  (P0-10 does this by hand first).
- Tests: every key a handler accepts appears in the table; no key appears twice in one scope; the
  generated README keys table matches the committed one (`fbittorrent robot-docs keys` prints it);
  `plugin/oliver.torrents/keymap.json` is generated from the same table so the dashboard's
  `KeyHint` rows cannot drift (P4-02).
- Why: footer, help and handlers are three hand-kept copies today (§1.7), and Zellij's release
  notes name exactly this drift as a source of confusion [UX "action legend"].

### 5.5 Visual language for the TUI

The TUI cannot use the shell's components, but it can use the same palette and the same rules.
`Theme::load` reads `colors.toml` (`src/tui/theme.rs:57-79`); `palette.rs` turns it into roles.

#### 5.5.1 Colour roles and the contrast rule

| Role | Source key(s) | Rule |
|---|---|---|
| text | `foreground` | as is |
| dim (headers, hints, secondary) | `foreground` and `background` through the ladder of §4.2.1 (the only definition) | **must reach 4.5:1 against `background`**, which the ladder guarantees; on the owner's theme rung 1 gives `#b6b6b6` (10.4:1), the same hex the shell's own panels use. Fixes D16: today `theme.rs:66-67` loads `muted` and then overwrites it with `dark_foreground` (`#505050`, 2.6:1); neither key is used for text any more. |
| accent (focused border, keys, sorted header) | `accent` | if `accent` and `foreground` differ by less than 32 in every RGB channel **and** are within 1.3:1 of each other (`kanagawa`, whose accent equals its foreground, qualifies; the owner's `#8d8d8d` against `#ffffff` does not: the channels differ by 114 and the luminance ratio is 3.3, so the grey accent stays distinguishable and is used; tokyo-night's blue accent on a grey-white foreground, luminance ratio 1.19 but channel difference 47, does not either, because hue carries the difference), keys use bold `foreground` instead of accent; and whenever `accent` is below 4.5:1 against `background` (`catppuccin-latte` 4.3, `miasma` 3.9, `rose-pine` 3.1), keys and the ready badge use bold `foreground`. A luminance-only rule ("within 2.0:1") would strip accent from most dark themes and make the TUI look less like the shell's own panels |
| selection (selected row background) | `selection` | the selected row is also marked by `▌` in column 0 (exists today), so the background is reinforcement |
| ok / warn / err | `green` / `yellow` / `red` | **used only as reinforcement**; when the theme is low-chroma (§5.5.2) they collapse to `text` and the glyph/word carries the meaning |
| border | dim; focused pane `accent` | as today (`theme.rs:85-86`) |
| poster placeholder | `selection` | the half-block fallback uses real pixels |

A contrast test (P4-05) loads every `colors.toml` under `/usr/share/omarchy/themes/` plus the
owner's current one and asserts the computed dim reaches 4.5:1 and `foreground` reaches 4.5:1 on
each (`rose-pine` is 6.7:1, so 7:1 is not a gate the TUI can meet; the test prints the ratio so
a theme below 7:1 is visible in the log). The same theme fixture (`contracts/ui/themes/`) feeds
`contrast.py` and `qs-format.sh::dim_ladder` (P2-08), so QML and TUI agree on the dim colour of
every theme to the hex.

#### 5.5.2 Low-chroma detection

A theme is "low-chroma" when the maximum channel difference between `red` and `green` is below
32 (the owner's theme: `#a4a4a4` vs `#b6b6b6`, difference 18). On such themes: error rows are
bold with a leading `!`, success feedback is plain text with a `✓`, errors in the status row are
bold with `!`, and the seed/rating colouring is dropped. Nothing else changes, because every state
already has a glyph and a word (§5.5.3).

#### 5.5.3 The state column

Every list row in the TUI has a state cell of glyph + word, width 9, never hidden at any width.
The glyphs are width-1 in JetBrainsMono Nerd Font (checked by `display_width()` in a test, so a
wide glyph can never shift columns):

The cell is the "Text glyph" and `short` columns of §4.3.4's table, which `format.rs` reads from
the same `contracts/ui/states.json` as `Model.js`: `◌ Meta`, `⟳ Check`, `↓ Down`, `~ Stall`,
`… Queue`, `‖ Pause`, `✓ Done`, `↑ Seed`, `⏚ VPN`, `! Error` (bold, the message in the details
strip), `! Disk` (P5-07), and `▶ Ready` replacing `↓ Down` while the torrent is both downloading
and playable. A spinner runs in the footer while metadata resolves. `format::tests::states`
asserts every cell is exactly 9 cells wide after padding and every text glyph has display width 1.

These reuse the vocabulary the Shows screen already uses for releases (`↓ … ‖ ✓ +`,
[UX "new, unread, added before and complete"]), so one glyph means one thing everywhere.

#### 5.5.4 Typography and borders

- Weight: bold only for the pane title, the selected row, keys and errors. Reverse video only for
  the active tab and for errors on low-chroma themes.
- Borders: rounded (`BorderType::Rounded`) as today; the focused pane in accent, others dim; modals
  rounded with a title in the border.
- Column headers: dim, uppercase, as today, but with `format.rs` widths so numbers align in every
  pane.
- Marquee: gone (P4-06). The selected row's full name is the second footer row; hover scrolls once
  and stops.
- Numbers: identical rules to `Model.js` (§4.2.3) via `src/tui/format.rs`, shared with the CLI's
  human output so `fbittorrent list` and the TUI print the same strings.

#### 5.5.5 Poster card rules

- Posters are 24×18 cells (`src/tui/poster.rs:12-14`), rendered through the kitty graphics
  protocol when the terminal supports it (Ghostty does) and as half-block cells otherwise.
- The card appears when the pane is at least `CELL_W + 20` columns wide (`src/tui/mod.rs:2223`);
  narrower panes show text only. On the narrow layout `v` toggles a full-width card overlay for
  the selected row.
- A kitty placement is removed whenever a modal, the help or a prompt covers it (today's
  `overlay` check, `src/tui/mod.rs:2229`), and re-placed after.
- The card's text: title and year (bold), kind, `★ 7.9 · 102k votes` (dim, not yellow on
  low-chroma themes), stars, blurb, and the `i`/`w` hints; the Shows card takes the height it
  needs instead of a fixed 9 rows ([RC §9] lower-severity note).
- Lookups are debounced (300 ms after the cursor rests) and cached per release name, as today.

### 5.6 Responsive layouts

Breakpoints by terminal width: narrow below 100 columns, medium 100 to 139, wide 140 and above.
Today the Torrents details pane needs 140 columns and the Search pane 100
(`src/tui/mod.rs:2770-2785`); the owner's tiled window is about 90 columns.

| Screen | Narrow (80) | Medium (120) | Wide (160) |
|---|---|---|---|
| Torrents | State (9), Name, %, ↓ when live, ETA. No side pane; Enter opens details full-screen (Overview first, as the mockup below, Appendix B and the dashboard; `]` reaches Files), `h`/Esc returns. | Adds Size and Peers, and a one-line strip above the footer with the selected torrent's state, reason and path. | Side pane with the IMDb card and details, as today, plus the State column. |
| Search | Name, Size, Seeds, IMDb ★; poster card toggled with `v` | Adds Uploaded and Provider | Side pane with poster; Uploaded, Source and Peers stay visible (today they hide whenever the pane shows, `src/tui/mod.rs:3000,3023`) |
| Shows | One pane with a breadcrumb ("Shows › It's Always Sunny"); Right/`l` enters, Left/`h` leaves; the episode key column never truncates | Two panes, card under the list | Two panes and the IMDb card at full height |
| RSS | One pane with breadcrumb; episode key kept at the left of the title, the middle elided; size column only when sizes are known | Two panes | Two panes |
| Details (modal, all widths) | Tabs Overview · Files · Peers · Log (`[`/`]`), full screen | as a modal of 100×30 | the side pane plus the same modal on Enter |

State is never colour-only at any width: the state column always carries a word and a glyph.
Mockups follow; Appendix C has the rest.

**Torrents at 80×24 (narrow):**

```
[ 1 Torrents ]  2 Search   3 Shows (2)   4 RSS              ↓ 3.1 MB/s ↑ 32 KB/s
╭ Torrents (4) ────────────────────────────────────────────────────────────────╮
│ STATE     NAME                                        %       ↓    ETA       │
│ ▌↓ Down   The Diplomat S04E01 1080p WEB-DL H26…      43%    3.1M    12m      │
│  ▶ Ready  Some.Movie.2026.1080p                      18%    2.0M    18m      │
│  ◌ Meta   Other.Movie.2026.1080p                      —       —      —       │
│  ✓ Done   The Diplomat S04 (2026) 1080p WEB…        100%                     │
│                                                                              │
╰──────────────────────────────────────────────────────────────────────────────╯
[Torrents] (Enter) Details  (O)pen  (P)ause  (U) Seed  (D) Remove  …  (?) Help
The Diplomat S04E01 1080p WEB-DL H264-GROUP · downloading · 41 peers
```

**Torrents at 120×30 (medium):**

```
[ 1 Torrents ]  2 Search    3 Shows (2)    4 RSS                                        ↓ 3.1 MB/s ↑ 32 KB/s
╭ Torrents (4) ────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ STATE     NAME                                              SIZE       %        ↓     ETA  PEERS                 │
│ ▌↓ Down   The Diplomat S04E01 1080p WEB-DL H264-GROUP      1.1 GiB    43%    3.1M    12m     41                  │
│  ▶ Ready  Some.Movie.2026.1080p                            2.1 GiB    18%    2.0M    18m     12                  │
│  ◌ Meta   Other.Movie.2026.1080p                                —       —       —      —      3                  │
│  ✓ Done   The Diplomat S04 (2026) 1080p WEBRip x265        3.4 GiB   100%                                        │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 downloading · queue 1/3 · 32 KiB/s cap, stops when complete · ~/Downloads/The Diplomat S04E01…
[Torrents] (Enter) Details  (O)pen  (P)ause  (U) Seed  (D) Remove  (Shift+D) Delete files  (A)dd  (R) Recheck  (?) Help
(1–4) Screens  (Tab) Switch screen  (↑↓) Move  (s) Sort  (/) Filter
```

**Torrents at 160×45 (wide)** keeps today's layout with the side pane
(`screens/01-torrents-160x45.txt`), adds the STATE column, keeps Size and Peers visible, and shows
the "Ready to watch · 4 min buffered" line in the details pane.

**Details modal at 80×24 (Enter on the narrow layout):**

```
╭ The Diplomat S04E01 1080p WEB-DL H264-GROUP ─────────────────────────────────╮
│ [Overview]  Files  Peers  Log                                     [ / ] tabs │
│ state      ↓ Down · downloading                                              │
│ ready      ▶ 4 min buffered · (O)pen                                         │
│ progress   43%  486 MiB of 1.1 GiB      eta   12 min                         │
│ speed      ↓ 3.1 MB/s  ↑ 32 KB/s        peers 41 (seeds unknown)             │
│ upload     32 KiB/s cap · stops when complete  (u to seed)                   │
│ path       ~/Downloads/The Diplomat S04E01 1080p WEB-DL H264-GROUP           │
│ added      12:41 today                  origin  search (yts)                 │
│ imdb       The Diplomat (2023) · TV series · ★ 8.0 · 102k votes   (v) poster │
│            Keri Russell, Rufus Sewell                                        │
│            Political thriller: a veteran US diplomat is appointed ambassa…   │
╰──────────────────────────────────────────────────────────────────────────────╯
[Details] (O)pen  (P)ause  (U) Seed  (D) Remove  (i) IMDb  …          (Esc) Back
```

### 5.7 The 15 UX defects, plus two

| # | Defect [RC §9] | Fix | Item |
|---|---|---|---|
| 1 | Below 140 columns state, peers, path and the IMDb card vanish; state is colour-only | Permanent state column; selected-torrent strip at 100-139; full-screen details below 100 | P4-04 |
| 2 | Red and green collapse on the monochrome theme | Glyph and word for every semantic state; low-chroma detection (§5.5.2) | P4-05 |
| 3 | Dimmed text `#505050` on black (about 2.6:1) | The dim ladder of §4.2.1, applied by `palette.rs` (§5.5.1) | P4-05 |
| 4 | RSS at 80 columns loses the episode id; a "0 B" size column | Keep the episode key at the left of the title and elide the middle; drop the size column when sizes are unknown | P4-07 |
| 5 | Destructive confirm defaults to yes | Only `y` confirms; Enter and Esc cancel; label "y/N" | P0-13 |
| 6 | Quitting stops all transfers without warning | `q` closes the UI; `Q` confirms when transfers are active | P1-15 |
| 7 | Resume on a finished torrent does nothing and says "started" | Hide Resume for `done`; offer Seed; the API returns a typed error | P0-10 |
| 8 | SPEED shows the size for inactive torrents | The rate column is blank when not downloading; Size is its own column | P4-07 |
| 9 | Misleading counts ("0 seeds", "0s", YTS "0" seeds, RSS "0 B") | Unknown is absent everywhere | P0-11, P3-01, P4-07 |
| 10 | The selected name marquee-scrolls forever and wraps | No marquee on the selected row; its full name shows in the second footer row; hover scrolls once and stops at the start; `reduce_motion = true` disables scrolling | P4-06 |
| 11 | The footer drops actions silently at narrow widths | Priority packing and an overflow cue `…` | P4-02 |
| 12 | Key meanings change by screen | One keymap, Appendix B (DL-34), enforced by the action table of §5.4 (`no_duplicate_key_per_scope`, `global_keys_never_rebound`) | P4-02, P4-07 |
| 13 | Feedback disappears after 8 or 15 s and is not logged | Message history plus a Log modal (`L`) fed by the daemon log; ftui `Toast`/`NotificationQueue` for transient feedback | P4-08, P1-14 |
| 14 | Duplicates, mojibake, hidden columns at 160, a stale query over browse lists | Daemon dedupe and repair; columns kept at wide widths; browse mode shows the list name in the query box | P3-01, P4-07 |
| 15 | Widget removes without confirm; widget hidden while closed | Confirm strip; `showWhenStopped` setting | P0-13, §4.5 |
| 16 | Dim prefers `dark_foreground` over `muted` (D16) | §4.2.1 ladder via §5.5.1; neither key is a text colour | P4-05 |
| 17 | The tab bar shows a meaningless theme name (D17) | Drop it; the daemon state ("idle · stops in 4 min", "waiting for VPN") takes the rate text's place while both rates are 0 or the engine is parked (the freed 8 cells are not the 21 the text needs at 80 columns) | P4-07 |

Lower-severity notes [RC §9], all in P4-07 unless noted: put the in-app browse lists before the
Cloudflare-blocked 1337x entry (`src/browse.rs:34-36`); natural sort in the files modal; F6 gains
save path, seed default, idle exit and limits (P5-04) and the peer port (P6-09); PgUp/PgDn and
Home/End on every list; the Shows IMDb card takes the height it needs; "(F)ollow show" only for TV
titles; the desktop icon (P1-13); `shows.json` permissions (P0-14).

### 5.8 FrankenTUI widgets the TUI uses

All in the pinned 0.6 crates (registry paths in §1.7), so no upgrade is needed for M3:

| Need | Widget | Where it lands |
|---|---|---|
| Torrent, search, release and feed lists | `ftui_widgets::table` for rendering, header hit-testing (`TableState::handle_mouse`) and the selection/offset state; **sorting is done by the TUI's store on typed values before building rows** (`sort_column: None`), because the widget's own sort compares cell text (`ftui-widgets-0.6.0/src/table.rs:228-259`) and would order "900 MiB" after "1.1 GiB" and "12m" after "1h05". Only the visible slice of a long result list is turned into `Row`s; `virtualized` supplies the scroll arithmetic | P4-04 |
| Screen tabs | `tabs` (as today) | unchanged |
| Transient feedback with history | `toast` + `notification_queue`; history in the Log modal | P4-08 |
| Log modal | `log_viewer` with level/source filters | P4-08 |
| Speed sparkline in the tab bar | `sparkline` | P6-03 |
| Pieces bar | a one-line `canvas` (ftui-extras) row | P6-02 |
| Posters | `ftui_extras::image` (kitty) and `canvas` (half-block), as today | unchanged |
| Unread badges on tabs | `badge` | P4-07 |
| Scrollable panes | `scrollbar` on every list (visible when content overflows) | P4-04 |

Not used on purpose: `command_palette` (the owner asked for no palette until a task needs it,
[UX "The next changes"]), drag-and-drop, markdown.

### 5.9 TUI tests

- The 21 interaction tests are kept and extended for every behaviour change.
- Golden renders for each screen and modal at 80×24, 120×30 and 160×45, stored as text in
  `src/tui/tests/golden/`, created in P0-16 from the unsplit code and updated only deliberately
  with `FBT_UPDATE_GOLDEN=1`. A golden test prints a unified diff on failure.
- Action-table invariants (§5.4).
- The contrast and low-chroma tests over every installed theme (§5.5.1, §5.5.2).
- A width test: every state glyph of §5.5.3 has `display_width() == 1`.
- The native tmux smoke at three sizes against a disposable daemon (P4-10) replaces the legacy
  `tui_smoke.sh`, which needs qBittorrent; each step logs a JSONL line with the captured pane
  (§9.1).

---
## 6. Feature roadmap toward meaningful qBittorrent parity, and beyond it

### 6.1 Tiers

- **T1, must-have:** fixes a lie the UI tells today, or directly serves workflows W1 to W5, and costs
  at most M once the daemon exists. Includes the OmaTorrent capabilities the owner named (turtle
  mode, notifications) and the streaming-first capability this product exists for (ready to watch).
- **T2, should:** helps diagnose a slow or stuck download, or adds comfort; may need a change in the
  rqbit fork.
- **T3, later:** marginal for this owner, or large engine work.
- **Out:** decided against for this owner; §2.6 has the reasons and "revisit if" triggers.

Rule for both UIs: T1 lands in the dashboard **and** the TUI. T2 and T3 land in the dashboard first
and in the TUI when the work is small (A2). Every In feature lands API-first with a fixture (§2.2
principle 6) and a CLI verb or flag (Appendix F), so agents get it at the same time as the UIs.

### 6.2 Decision table: the 18 missing and 9 partial features, re-examined

Round 1 re-read every row against §2.1 and §2.7. Rows whose decision changed are marked
"(changed)"; new rows are marked "(new)".

| # | Feature [RC §8] | Today | Decision | Tier | Why, for this owner | Item |
|---|---|---|---|---|---|---|
| 1 | Recheck | implicit on every open; on start after an error | In | T1 | After a crash, a disk swap or a doubt about data, the user needs an explicit check; the engine already rebuilds and re-checks (`src/engine.rs:756-760`). | P5-03 |
| 2 | Reannounce | no route | In | T2 | Useful when a download stalls; needs a fork change in tracker comms. | P6-01 |
| 3 | Rename | none | Display name only | T3 | Cosmetic. Renaming payload files is Out (§2.6). | P7-03 |
| 4 | Move / set location | only at add (`src/main.rs:513`) | In | T3 | Files can be moved with a file manager after removing the torrent; becomes cheap once fast resume avoids a re-check. | P7-02 |
| 5 | Global speed limits | none (`src/engine.rs:417-439`) | In | T1 | Streaming next to a video call; `Session` exposes `pub ratelimits: Limits` (`vendor/rqbit/librqbit/src/session.rs:140`) with live setters `set_download_bps`/`set_upload_bps` (`limits.rs:85-91`), so no fork change is needed. | P5-04 |
| 6 | Alternative limits / scheduler | none | Turtle In; scheduler Out | T1 / Out | omaqbt's `z` is the most-used control in that plugin's design [FO §5]; a schedule does not fit an on-demand daemon. | P5-04 |
| 7 | Super seeding | none | Out | - | Seeding is off by default. | - |
| 8 | Peers view | count only, mislabelled | In | T1 | Diagnoses slow downloads and makes the seed count true; `per_peer_stats_snapshot` exists (`vendor/rqbit/librqbit/src/torrent_state/live/mod.rs:782`), `PeerStats` carries counters, state, connection kind and client name (`live/peer/stats/snapshot.rs:27-32`). | P5-01, P5-02 |
| 9 | Trackers view | none | Status view In (T2); add and remove (T3) | T2 / T3 | The tracker list is known (`torrent_state/mod.rs:186`); per-tracker status needs a fork change. | P6-01, P7-04 |
| 10 | Speed graphs | current rates only | In | T2 | A ring buffer is cheap; ftui 0.6 ships `sparkline.rs`; the dashboard draws the same points. | P6-03 |
| 11 | Execution log | 8/15 s status line; `last-error.txt` | In | T1 | Replaces ephemeral feedback ([RC §9] #13) and the hidden error file. | P1-14, P3-15, P4-08 |
| 12 | RSS auto-download rules | none, by design (`README.md:37`) | Out | - | Owner decision: RSS is feeds and items; principle 2 (deliberate downloads); Shows covers following. | - |
| 13 | Torrent creator | library code unexposed | Out | - | §2.6 | - |
| 14 | IP filter | none | Out | - | §2.6 | - |
| 15 | Proxy (SOCKS/HTTP) | none | Out | - | §2.6 | - |
| 16 | DHT/PeX/LSD toggles | always on | UI Out; honour the private flag | Out / T2 | Public swarms need discovery. Private torrents (BEP 27) must not use DHT or PeX; P6-12 verifies the engine honours the flag. | P6-12 |
| 17 | Remote backend | legacy WebUI URL only | Out; legacy mode removed in v0.4.0 | - | §2.6; SSH plus the TUI covers remote use. | P4-09 |
| 18 | Completion notifications | only "added" and "add failed" (`src/main.rs:527-623`) | In | T1 | "Tell me when it is ready to watch"; the daemon owns it. | P1-13 |
| 19 | File selection and priorities | include/skip only | Keep include/skip; better Files view | T1 | Natural sort, per-file progress and open-file serve W1; high/max stay Out (§2.6). | P3-10, P4-07 |
| 20 | Pause/resume/force-start | no force start, no reorder | Force start and queue order In | T2 | "Watch this one now" when the three slots are busy. | P6-05 |
| 21 | Categories | passed through, no UI | UI Out; an `origin` field instead | T2 | One download folder; "where did this come from" (search, RSS, Shows, manual) is the useful grouping. CLI `--category` keeps working for agents. | P6-08 |
| 22 | Tags | only the internal `seed` tag | Out for UI | - | The API hides tags behind `seed: bool`; users never see the tag. | - |
| 23 | Per-torrent speed limits | upload cap only; a change rebuilds and re-checks (`src/engine.rs:766-776`) | Live upload cap (T2); download cap (T3) | T2 / T3 | Removes a surprising re-check; download caps are rarely needed with global limits. | P6-07, P7-06 |
| 24 | Share ratio / seed time | unlimited or stop at completion | Keep as is | - | Seeding is off by default; `seed` covers the exception. | - |
| 25 | Piece map | progress bar | Pieces bar In | T2 | Shows that sequential and first/last pieces are working, which is what streaming needs; the same bitfield feeds ready-to-watch. | P6-02, P5-06 |
| 26 | Statistics | served by `transfer/info`, never shown | In | T2 | Session and all-time totals; cheap. | P6-04 |
| 27 | Port / UPnP / uTP | `peer_port` in config only; UPnP refused by the fork (`session.rs:569-570`); TCP only | Port in Settings (T2); UPnP Out; uTP Out | T2 / Out | With VPN binding, UPnP is moot; without it the owner is a leecher-first client that reaches peers through DHT and outgoing connections; uTP is large engine work. | P6-09 |
| 28 (new) | Ready to watch | nothing tells the user when streaming can start | In | T1 | The product's reason to exist (A6, §2.7). | P5-05, P5-06 |
| 29 (new) | Show follow quality per show, inbox filters | `Show.quality` exists (`src/shows.rs:62-64`) but only the CLI sets it | In | T1 (dashboard) / T2 (TUI) | The owner follows shows at 1080p by default and wants 2160p for some. | P3-18, P4-07 |
| 30 (new) | Poster-rich browse grid | browse lists are text rows (`screens/10-browse-yts-latest.txt`) | In | T2 | The lists carry IMDb ids, so posters are free; a grid is how people pick a film. | P3-17 |
| 31 (new) | Clipboard magnet watch | none | In, opt-in, off by default | T2 | Fragments' best idea [FO §5]; A7. | P6-13 |
| 32 (new) | Undo remove (keep files) | none | In | T2 | W8; a mis-press should cost nothing. | P6-08 |
| 33 (new) | Agent event stream and raw API verb | agents poll `triage` | In | T1 | `events --follow` ends polling; `api` exposes every operation to scripts without waiting for a verb. | P1-18 |
| 34 (new) | Tracker scrape for swarm-wide seeds | unavailable ([EE] "Swarm statistics") | Out | - | §2.6; DL-19. | - |
| 35 (new) | Web seeds | none | Out | - | §2.6 | - |
| 36 (new) | Sleep inhibitor while downloading | none | In | T2 | A laptop must not suspend mid-download [FO §5]. | P6-09 |
| 37 (new) | Low disk space warning and `disk_full` state | none; `ENOSPC` becomes a raw engine error | In | T1 | The likeliest failure after the VPN for a streaming user with a 3.4 GiB library on a laptop; qBittorrent, Fragments and Transmission all warn before it happens. | P5-07 |
| 38 (new) | Subtitle fetch | none | Out | - | OpenSubtitles needs an account and an API key, which the owner excluded (§2.5); the player's own subtitle search covers it. | Revisit if a keyless source appears |
| 39 (new) | Quiet hours for notifications | none | Out (nothing to build) | - | The shell's notification service owns Do Not Disturb (`plugins/bar/indicators/Dnd.qml`), and `notify-send` goes through it. | - |
| 40 (new) | Player command | `xdg-open` | In | T2 | `open_command = "xdg-open"` in `Settings` (A.4); mpv users set `mpv --keep-open=yes` so a growing file does not end playback early. One config key, no UI beyond Settings. §10.3 Q15. | P3-15 |

Additional items that are not in the qBittorrent inventory but matter here:

| Item | Tier | Why | Work item |
|---|---|---|---|
| Lazy restore of completed torrents (no hash check at daemon start) | T1 | Under A1 the daemon starts often; checking finished data every time wastes minutes on a large library (D3). | P1-16 |
| Staged add with file selection and seed switch | T1 | "Deliberately download" (§4.8). | P3-06, P3-11 |
| App-owned fast resume for all torrents | T2 | Removes the remaining start-up checks safely. | P6-06 |
| Bulk remove of a filtered set; undo for remove-keep-files | T2 | W8 cleanup. | P6-08 |
| New-release notifications for Shows | T2 | Closes the follow-a-show loop without opening the app. | P6-10 |
| Destination picker with free space | T2 | Avoid filling the disk; Quickshell has no file dialog [FO §2.11]. | P6-11 |
| Torznab indexers (Jackett, Prowlarr) | T3 | Private trackers without Python; marketplace-friendly (§7). | P7-01 |
| Quick-add and search overlay palette | T3 | A3's deferred overlay. | P7-05 |
| Deferred add while waiting for the VPN | T3 | Today an add while parked fails (`src/engine.rs:830-832`); a magnet could be saved and resolved later. | P7-09 |

### 6.3 Specifications of the In features

Each feature lands API-first (principle 6). The table names the daemon change, the API surface,
both UIs, the CLI verb and the test. Items without a CLI column entry are reachable through
`fbittorrent api <op>` (P1-18) until they get a verb.

| Feature | Daemon / engine | API | TUI | Quickshell | CLI | Test |
|---|---|---|---|---|---|---|
| Completion notifications (P1-13) | Emit `torrent.completed` where the queue loop first sets `completion_on` (`src/engine.rs:301-304`); notifier thread with actions | event `torrent.completed {hash, name, open_target}` | feedback row "Completed: <name>" | Done group updates; the OS notification does the rest | `events --follow` prints the event | `daemon.py::notify_completed`: local magnet fixture completes; a fake `notify-send` on `PATH` records its argv; assert one call with both actions; `notify_action_open` feeds "open" on the fake's stdout and asserts a fake `xdg-open` received the file path |
| Added notification with Undo (P1-13, A5) | after `torrents.add` with `origin: "handler"` (the CLI's TTY toast is plain, without Undo, P1-13) | event `torrent.added {hash, name, origin}` | n/a | n/a | the handler path | `daemon.py::notify_added_undo`: "undo" on the fake's stdout removes the torrent, keeps files |
| Execution log (P1-14, P3-15, P4-08) | Ring of 2,000 entries; journald copy | `app.log {since_seq?, limit?, level?, hash?}`, topic `log` | Log modal `L`; message history | Log view; Log tab per torrent | `log [--follow] [--json]` | Unit: ring bound and ordering; `daemon.py::log_injected_error` sees an injected error once with its source |
| Agent event stream and raw API (P1-18) | none | existing | n/a | n/a | `events [--follow] [--since-seq N] --json` prints NDJSON events (one JSON object per line on stdout, exit 0 on EOF, 4 when the daemon is down); `api <op> [--arg k=v…] [--json-args '{…}']` | `agent_contract.py::events_follow_completion`: a completion arrives within 2 s of the fixture finishing; `api_unknown_op` exits 2 with the hint |
| Seed count and peer list (P5-01, P5-02) | Fork: add `seed: bool` to `PeerStats` (`torrent_state/live/peer/stats/snapshot.rs:27-32`) from the live peer's `bitfield: BF` (`live/peer/mod.rs:259`; the all-pieces test `has_full_torrent` at `:297-299` already exists) in the `From<&Peer>` impl at `snapshot.rs:57`; the daemon derives per-peer rates from successive `counters` samples | `torrents.peers {hash}`, topic `torrent:<h>.peers`; `seeds` (connected seeds) in `Torrent` | Details tab Peers | Details tab Peers; list column "seeds/peers" | `show <sel> --json` gains `seeds`, `peers[]` | Fork unit test with a local seed and a local leecher; fixture `peers.ndjson`; UI golden renders |
| Recheck (P5-03) | `torrents.recheck` uses the existing rebuild path (`src/engine.rs:756-760`); invalidates fast-resume data (P6-06) | op, pending until the check ends | `R` on Torrents, confirm "Recheck '<name>'? It re-reads all data. y/N" | `R` and an Overview button, same confirm | `recheck <sel>…` | `daemon.py::recheck_corrupt_byte`: corrupt one byte of a local payload, recheck, see progress below 100%, then a re-download from the local seed restores it |
| Global and alternative limits (P5-04) | `session.ratelimits.set_download_bps/set_upload_bps` live; config keys with the `Settings` names of A.9: `download_limit_bps`, `upload_limit_bps`, `alt_download_limit_bps`, `alt_upload_limit_bps`, `alt_limits_on` (so the turtle state survives a restart; `summary.alt` mirrors it). The global upload cap never lifts the per-torrent 32 KiB/s default | `transfer.set_limits`, `transfer.set_alt {on}`; `summary.alt`, `summary.limits` | `z` toggles; F6 edits; tab bar shows the turtle glyph when on | `z`, popup hero button, dashboard top bar indicator, Settings sheet | `limits [--down N] [--up N] [--alt on|off] [--alt-down N] [--alt-up N]` | Unit: limits applied to `Limits`; `daemon.py::alt_limit_holds`: with a 64 KiB/s alt limit a local transfer stays within ±25 % over 20 s; `daemon.py::alt_survives_restart` |
| Ready to watch (P5-05, P5-06) | Fork: a `pub fn have_pieces_snapshot(&self) -> Option<BitVec>` on `ManagedTorrent` wrapping `with_chunk_tracker` (`torrent_state/mod.rs:266`, `chunk_tracker.rs:205`). Daemon: once per second per live incomplete torrent, for the largest video file compute the contiguous byte prefix from the file's first piece; `ready` when prefix ≥ max(32 MiB, 3 % of the file) and the last piece of the file is present; `buffered_s` = prefix ÷ (file size ÷ duration) when a duration is known (from `release.rs` is not enough; a duration is read from the file's container header only if cheap, otherwise omitted); fire `torrent.playable` once per torrent per daemon life; notify once | `playable {file, ready, stall_free?, finishes_in_s?, buffered_s?, prefix_bytes, acked}` in `Torrent` (`stall_free` = `prefix_bytes / bitrate ≥ (file_size − prefix_bytes) / dl_bps`, recomputed per second); event `torrent.playable {hash, name, file, open_target}`; `wait --until playable` and `--until stall-free` map to them | state cell `▶ Ready`; details line; `o` opens | row badge; sidebar "Ready"; bar icon state; popup [Open] | `wait <sel> --until playable`; `events` shows the event; `show` carries `playable` | `daemon.py::playable_before_complete`: a local seed throttled to 1 MiB/s serves a 64 MiB fixture `.mkv`; the event fires with `prefix_bytes ≥ 32 MiB` before `completed`; `playable_never_for_iso`: a `.iso` fixture never gets `playable` |
| Better Files view (P3-10, P4-07) | `torrents.files` adds per-file `progress` (exists) and natural-sort keys | topic `torrent:<h>.files` | natural order; `o` on a file | same | `show --json` files are in natural order | Fixture with E01..E10 out of order |
| Tracker status and reannounce (P6-01) | Fork: per tracker URL record the last announce time, its result (`ok` or the error message), the peers returned and the next announce time; `ManagedTorrent::tracker_stats() -> Vec<TrackerStat>` and `announce_now(url?)` | `torrents.trackers {hash}` → `{trackers: [Tracker]}` (A.9: `url`, `tier?`, `status: ok\|error\|updating\|not_contacted`, `last_announce_at?`, `next_announce_at?`, `peers?`, `message?`); topic `torrent:<h>.trackers` (keyed by `url`, 5,000 ms); `torrents.reannounce {hashes}` → `{state: "done"}`; `TorrentDetail.tracker_count`; cap `trackers` | Trackers tab in the details modal (URL, status, last announce, peers, error); `e` reannounces | Trackers tab in `DetailsPane.qml`, same columns; `e` | `reannounce <sel>…` (exit 0, 4, 6, 7) | `daemon.py::{tracker_status, reannounce}` with a local HTTP tracker fixture that errors for its first N announces and then succeeds, logging each announce; fork unit test `tracker_stats_records_error_then_ok` |
| Pieces bar (P6-02) | Base64 the have bitfield (P5-06's accessor); mark first and last pieces per file; mark the playable prefix | `torrents.pieces {hash}` → `{count, piece_size, have: base64, priority: [...], playable_end?}`; topic `torrent:<h>.pieces` | one-line bar in details | `PiecesBar` component | via `api` | Unit: bitfield round-trip and a 200,000-piece frame under 64 KiB; fixture with 1,000 pieces |
| Speed history (P6-03) | One ring of 600 one-second global samples (`download`, `upload` bytes per second); no per-torrent ring (no view draws one) | `Summary.history: [u64]` (the last 60 download samples), `app.stats {history: true}` → `Stats.history {download, upload, interval_s: 1}`; topic `stats` (5,000 ms); cap `history` | ftui `sparkline` in the tab bar (§5.8), redrawn on data | `components/Sparkline.qml` in the status row (§4.6), no interpolation (§4.13) | `status --json` gains `history` | Unit `history::tests::ring_bound_and_order`; TUI golden at 120×30 with a throttled ramp; `qs-smoke.sh::history_points`; `agent_contract.py::status_history_field` |
| Statistics (P6-04) | Session totals exist (`src/engine.rs:616-623`); all-time totals `{downloaded, uploaded, completed}` in `~/.local/state/fbittorrent/stats.json` (0600, temp-fsync-rename), saved on change at most every 30 s and at drain | `app.stats` → `Stats {session, all_time, history?}` (A.9) | a footer line in the Log modal ("session ↓ 4.2 GiB ↑ 120 MiB · all time ↓ 1.2 TiB ↑ 41 GiB · 312 completed", `format.rs` sizes) | a Stats section in the Settings sheet | `status --json` gains `all_time` | `daemon.py::stats_persist` (download, restart the daemon, totals unchanged plus the new session); unit `stats::tests::save_throttle_30s` |
| Force start and queue order (P6-05) | `Record.force: bool` (serde default `false`) skips the slot limit in `reconcile_queue` (`src/engine.rs:280-343`; `src/engine/queue.rs` after P1-02); the ledger order is the queue order and `queue_pos` is the index among incomplete, unforced, running-intent records; held torrents stay `queued` with `reason: queue_full` | `torrents.force_start {hashes, on}` → `{state: "done"}`; `torrents.queue_move {hash, to: top\|up\|down\|bottom}` → `{state: "done", queue_pos}`; `torrents.start {force: true}`; `Torrent.force`, `Torrent.queue_pos`; cap `queue` | `P` force start; `K`/`J` move up/down | `P`; Ctrl+k/Ctrl+j (`PanelKeyCatcher` `reorderable`) and `K`/`J` | `start --force <sel>`; `queue <sel> top\|up\|down\|bottom` | Unit `queue::tests::{admission_with_force, reorder_positions}`; `daemon.py::{force_start_bypasses_limit (`max_active_downloads = 1`, two fixtures, force the second: both run), queue_move_order}`; `agent_contract.py::{start_force, queue_verb}` |
| Fast resume (P6-06) | Fork: `initializing.rs` accepts a trusted have-bitfield (today it refuses, `torrent_state/initializing.rs:113-114`). The daemon writes `engine/resume/<hash>.bin` (the bitfield plus size and mtime of every included file) on clean drain and every 5 min for live torrents, and `engine/resume/CLEAN` as the clean-shutdown marker (removed at start, written at drain); a record is restored with the trusted bitfield only when the marker existed and every file's size and mtime match, else with a full check; `torrents.recheck` deletes the record's resume file first | none (behaviour); `checked: true` after a trusted restore | none | none | n/a | `daemon.py::{resume_trusted (restart with untouched data: no `checking`), resume_touched_file (touch one file: only that torrent re-checks), resume_after_kill9 (no marker: every torrent re-checks)}`; fork unit test `trusted_bitfield_accepted` |
| Live upload cap (P6-07) | Fork: expose the live torrent's `Limits` (`torrent_state/live/mod.rs:217`) through `ManagedTorrent::set_upload_bps(Option<u64>)` so a cap change applies without a rebuild (today `setUploadLimit` re-adds the torrent and it re-checks) | `torrents.set_upload_limit` answers `{state: "done"}` synchronously (`pending` until this item); `torrents.set_seed` uses the same path | none | none | `seed`/`noseed` stop re-checking | `daemon.py::cap_change_no_check` (a cap change never enters `checking`; the new cap shows in the leecher's rate within 5 s); fork unit test `live_limits_update` |
| Origin, filters, bulk remove, undo (P6-08) | `Record.origin` set by search grab, RSS grab, Shows grab, clipboard, handler (default `manual`); undo keeps the `.torrent` file and the record under `engine/undo/` for 10 s after a keep-files removal, in memory, per batch | `origin` field; `torrents.remove` with up to 256 hashes; `torrents.add {source: {restore: hash}}` within the undo window | `X` on a filtered view with count and size confirm; undo toast `U` (never `u`, which is seed) | sidebar "From:" filters; bulk remove; undo toast | `delete … --json` returns `undo_until`; `undo [<hash>]` (no hash: the last batch) | Unit: origin persisted; `daemon.py::undo_restore` re-adds and checks to `done` |
| Port setting and sleep inhibitor (P6-09) | `Settings.peer_port` (restart-class: the engine restarts in place, clients stay connected, §3.2.8); `src/daemon/inhibit.rs` holds one `systemd-inhibit --what=sleep --who=fbittorrent --why=Downloading --mode=block sleep infinity` child while any torrent is `downloading`, `stalled`, `metadata` or `checking` with intent `run`, kills it within 5 s of the last one stopping and on drain (§3.8) | `app.settings.set {peer_port}` | a field in F6 | a field in the Settings sheet | `settings --port N` | `daemon.py::{port_restart_keeps_clients, inhibitor_lifecycle}` (a `systemd-inhibit` shim records start and SIGTERM times) |
| Shows notifications (P6-10) | After a `shows.check` that found new unread releases: one notification per check through P1-13's notifier, title "3 new episodes", body with at most three release names then "+ N more", action `window` → `fbittorrent dashboard --view shows`; none when nothing is new; suppressed while a dashboard or TUI is interactive (the badge shows it instead) | none new (§3.8 row "new releases") | none | none | n/a | `daemon.py::{shows_notify, shows_notify_none_when_unchanged, shows_notify_suppressed_interactive}` with the EZTV fixture server |
| Add sheet destination (P6-11) | `statvfs` of the chosen folder; validate writable | `app.free_space {path}` → `{free_bytes?, writable}` (absent on error, never negative, §3.4.9) | the add prompt shows free space | a destination row ("312 GiB free") with `components/FolderPicker.qml`, a small directory browser (starts at `save_path`, Enter selects, `..` goes up, hidden folders skipped; Quickshell has no file dialog [FO §2.11]); an unwritable folder is refused | `add --save <dir>` validates (exit 2 when not writable) | Unit `free_space::tests::{absent_on_error, never_negative}`; `qs-dashboard.sh::destination_picker`; `agent_contract.py::add_save_unwritable` |
| Private flag (P6-12) | Verify the fork skips DHT and PeX for torrents with `private=1`; fix if not. Why verify: upstream threads `info.private` into the session (`vendor/rqbit/librqbit/src/session.rs:1115,1125,1365-1386`, `is_private`), but the fork replaced the runtime and the socket layer, and no app test covers the flag; a leak would announce a private tracker's torrent on the public DHT | none | none | none | n/a | A private fixture torrent with a local tracker: no DHT queries in the engine log |
| Clipboard watch (P6-13, A7) | Setting `clipboard_watch = false`; when on, the daemon runs `wl-paste --watch fbittorrent clipboard-event` is **not** used (it would spawn the CLI per change); instead the daemon runs one `wl-paste --watch cat` child with bounded output and parses magnets from its stream; dedupes by btih against the ledger and the last offer; never logs clipboard text | event `clipboard.magnet {hash, name, already_added}`; `app.clipboard.dismiss {hash}` | footer strip "Add '<name>'? (y/Esc)" | popup strip and dashboard banner [Add] [Dismiss]; notification with "Add" | `settings --clipboard-watch on|off` | `daemon.py::clipboard_watch_offer`: a fake `wl-paste` on `PATH` prints a magnet after 1 s; the event arrives; "add" adds it; a second identical magnet does not re-offer |
| Show quality per show, inbox filters (P3-18) | `shows.set_quality {imdb, quality?}` saves `Show.quality` (`src/shows.rs:62-64`) and the next check prefers it; the inbox gains filters (quality, unread only) through `shows.releases {inbox: true, unread_only?, quality?}` | op; `Show.quality` already in the object | `=` opens a quality prompt on a show row (P4-07; `Q` stays quit, Appendix B) | a `Dropdown` (2160p, 1080p, 720p, 480p, default) on the show row and a filter chip row on the inbox | `shows follow --quality`, `shows quality <show> <q>` | Unit `shows::tests::quality_persists`; `daemon.py::shows_quality_prefers`; `qs-dashboard.sh::shows_quality_dropdown`; `agent_contract.py::shows_quality_verb` |
| Poster grid (P3-17) | none (browse lists already carry IMDb ids, `src/qbt.rs:100-102`) | existing `search.browse` + `imdb.lookup` | n/a (lists stay rows in the TUI) | `BrowseGrid`: 3 to 6 columns of 2:3 poster cells with title, year, ★ rating and seeds; Enter stages the add; `v` toggles grid/rows | n/a | qs smoke with the poster fixture; a render-time budget assertion (no frame over 32 ms in the `QSG_RENDER_TIMING=1` log while 60 posters load, the P3-09 budget) |
| Torznab (P7-01) | Native provider kind `torznab` in `src/search.rs` (§7.3): queries `?t=search&q=<query>&apikey=<key>` with a 30 s timeout and a 4 MiB response cap; the key is read from a 0600 file opened with `O_NOFOLLOW` and refused when its mode is wrong (§3.9); a key over plain HTTP to a non-loopback host is `invalid_args`; a cross-origin redirect is `upstream`; the key never appears in logs or messages | `search.install {sources: [https URL]}`; `search.providers` lists `Provider.kind: torznab`; cap `torznab` | providers modal lists the kind | providers sheet lists the kind | `search --install-torznab <url> --key-file <path>` | `daemon.py::torznab_{results, http_key_refused, redirect_refused, key_not_logged}` (`fixtures.torznab_server()` with a redirect case); unit `torznab::tests::parse_fixture` |
| Move (P7-02) | Pause; move the payload (rename on the same filesystem; copy, fsync, unlink across filesystems; a failed copy leaves the source intact and reports `failed`); update `Record.output_folder`; restore with fast resume (P6-06) so a completed torrent stays `done` | `torrents.move {hashes, save_path}` → `{state: "pending"}` then `mutation.result` (not idempotent, A.1); `Torrent.save_path` follows; cap `move` | none in TUI (T3) | Overview "Move…" with the folder picker of P6-11 | `move <sel> <path>` (exit 0, 2, 4, 6, 7, 8) | `daemon.py::move_{same_fs, cross_fs, failed_copy_keeps_source}` (cross-filesystem through a tmpfs when available); `agent_contract.py::move_verb` |
| Rename display name (P7-03) | `Record.name` only | `torrents.rename {hash, name}` | prompt | inline edit | `rename <sel> <name>` | Unit |
| Tracker add/remove (P7-04) | Fork: a mutable per-torrent tracker set (`ManagedTorrent::add_tracker(url)`, `remove_tracker(url)`), persisted as `Record.trackers: Vec<String>` and merged with the `.torrent`'s list on restore | `torrents.add_tracker {hash, url}`, `torrents.remove_tracker {hash, url}` → `{state: "done"}` (`http(s)://` or `udp://`, else `invalid_args`); cap `tracker_edit` | Trackers tab: `+` prompt, `x` confirm (Cancel focused) | Trackers tab: `+`, `x` | via `api` | `daemon.py::tracker_add_remove` (two local tracker fixtures: the added one receives announces, the removed one none after the next cycle); fork unit test `tracker_set_mutation` |
| Quick-add palette (P7-05) | none | existing ops (`torrents.add` for a pasted magnet or path, `search.start` and the first ten `search:<id>` rows for text, `shows`-style staging on Enter) | n/a | a second plugin `plugin/oliver.torrents-quick/` with `kinds: ["overlay"]` and its own chord (an overlay kind must not share the bar-widget plugin [OR §6 gotcha 4]); one `FilterInput`; Esc closes; it reuses the `Service.qml` singleton through `serviceFor("oliver.torrents")` | n/a | `qs-smoke.sh::palette`; `qs-dashboard.sh::palette_summon_routing` (the bar plugin's toggle routing is unaffected) |
| Per-torrent download cap (P7-06) | uses P6-07's live limits; `Record.download_limit`; effective rate `min(per-torrent, global, alt)` | `torrents.set_download_limit {hashes, bps? ≥ 16384 or null}` | none | details Overview field | `limit <sel> --down N[K\|M]`, `--down none` clears | `daemon.py::download_cap_holds` (±25 % over 20 s after a 5 s warm-up); unit `effective_rate_is_min` |
| Deferred add while parked (P7-09) | While the slot is `Parked`, save a paused record from the magnet's btih and `dn=` name (or the `.torrent`'s metadata) instead of failing (`src/engine.rs:830-832`); resolve metadata and join the queue with the saved intent when the engine unparks; `{url}` sources stay refused while parked | `torrents.add` answers `{state: "done", hash, deferred: true}` (A.5); cap `deferred_add` | rows show `waiting_vpn` | same | `add` prints `deferred: true` | `daemon.py::deferred_add` with the namespace fixture (`vpn.py`, opt-in) and `deferred_add_saved_record` (always) |

---

## 7. Search policy and marketplace readiness

### 7.1 Decision: the Omarchy marketplace is not a goal

1. This is a personal tool for legal personal use in Switzerland (`README.md:271`).
2. The plugin depends on a native binary built from this repository, so a listing would at best
   carry the `manual-setup` label, and a listing needs a public repository with `manifest.json`
   at its root, while ours lives in `plugin/oliver.torrents/` [FO §2.10].
3. The marketplace reviewer treated a compiled-in list of indexers "whose primary purpose is
   indexing infringing copies" as a listing-policy blocker, and that question decided ucmz851's
   fate [OR §6 gotcha 7]. fbittorrent ships such a list on purpose: the curated provider URLs
   (`src/qbt.rs:223-237`, including the nyaa.si engine bundled by commit `c7d311f`), the provider
   browser for TPB, EZTV and YTS (`src/browse.rs:31-44`), and Shows built on EZTV. Removing them
   for a listing would remove features the owner uses.
4. The torrent space in the marketplace is crowded already [OR §1.3].

### 7.2 What the plan adopts anyway

These are good engineering whether or not a listing ever happens, and they are the review bar
ucmz851 hit [OR §8 item 10]:

- **Bounded reads everywhere.** Provider capture is already bounded (2 MiB, `src/search.rs:27`) and
  results are capped at 5,000 (`src/search.rs:26`); browse JSON is capped at 16 MiB
  (`src/browse.rs:60`). Unverified: explicit caps on RSS, IMDb and poster HTTP reads; P3-01, P3-02
  and P3-04 add them (4 MiB per feed, 2 MiB per IMDb or Wikipedia JSON, 8 MiB per poster) and test
  them with an oversized fixture.
- **Bounded output into the shell:** the line caps (§3.4.1), the 500-row search render cap and the
  60-poster grid cap (§4.15).
- **0600 files, written atomically, failing closed** (§3.9; P0-14).
- **Confirm before deleting data** (§4.9) and an explicit `delete_files` in the API.
- **Teardown on destroy:** the plugin owns no long-lived children (§4.15).
- **Screenshots match the real UI:** P2-08 and P3-16 capture them from the running shell, dark and
  light, never mockups [OR §6 gotcha 8].
- **Theme tokens only**, and no network access from QML.

### 7.3 Search sources

- qBittorrent nova plugins stay what they are: Python scripts the user installs with
  `fbittorrent search --install` (the curated list) or `--install-url`. The README already says
  they run with the user's rights (`README.md:213-216`). The daemon runs them exactly as the CLI
  does today.
- **Torznab (P7-01, T3):** a native provider kind for Jackett or Prowlarr, without Python.
  Configuration in `~/.config/fbittorrent/indexers.toml` (0600, opened with `O_NOFOLLOW`,
  refused if group- or world-readable): `[[torznab]] name, url, api_key_file` (the key lives only
  in its own 0600 file, never inline, so `settings`/`config` output can never print it; P7-01).
  Rules: HTTPS is required when an API key is set, except on loopback; redirects to another origin
  are refused; the key never appears in logs, errors or argv; responses are capped at 4 MiB and
  30 s (the numbers P7-01 and §6.3 implement); XML parsing uses `quick-xml`, already a dependency (`Cargo.toml:34`). Why it is worth
  having: private trackers through Prowlarr, and it is the reviewer-friendly kind of search
  [OR §8 item 10].

### 7.4 If the owner ever asks for a listing

Not planned (DL-17; P7-08 superseded in round 3). For the record, a listing would need: a separate
public plugin repository with a root `manifest.json`, README, licence and real screenshots; a build
without the curated providers and browse lists; the plugin detecting a missing or old binary; a
security note listing §7.2's bounds. None of it is designed further until asked.

---

## 8. Engineering hygiene

### 8.1 Landing the native engine on `main`

Order matters, because the working tree is the only copy of the product.

1. **Back up first** (part of P0-01): `tar --zstd -cf <temporary path>/worktree-2026-10-18.tar.zst --exclude=./target .`
   from the repository root, and record its SHA256 in the P0-01 receipt. Reading the tree is safe;
   nothing is modified. The copy is temporary (Q4: no archive) and is deleted once the landing
   commits exist.
2. **P0-01** moves the validation blobs out (§8.3). **P0-02** settles Asupersync (§8.2).
   **P0-03** fixes `.gitignore` and checks the nova3 licence.
3. **P0-04** commits on `codex/shows-native-engine` in this order:

| Commit | Content | Builds? |
|---|---|---|
| C0 `vendor: asupersync 0.5.0 with the fbittorrent patch` (only under §8.2 option B; otherwise this slot is empty) | `vendor/asupersync/` (57 MB) with its rewritten `FBITTORRENT_PATCH.md` and the licence file carried unmodified | n/a (not yet referenced) |
| C1 `vendor: rqbit 9.0.1 fork ported to Asupersync` | `vendor/rqbit/` (3.3 MB) with its provenance README and LICENSE | n/a (not yet referenced) |
| C2 `vendor: nova3 search runtime` | `vendor/nova3/` (64 KB) | n/a |
| C3 `app: native engine, IPC, VPN binding, queue, encryption, Shows, RSS, search, TUI` | all of `src/`, `Cargo.toml` (dependencies, workspace members, and the `[patch.crates-io] asupersync = { git = "…", rev = "<sha>" }` entry from P0-02), `Cargo.lock`, `README.md`, `CHANGELOG.md` starting at v0.1.1 (§8.5). One commit because the manifest, the lock and the sources that use them are one change; a `[patch]` for a crate the legacy manifest does not depend on is not a buildable intermediate state (`--locked` fails on a lock that names crates the manifest does not reference). The message's body carries the former "Asupersync 0.5.0 timer-wheel patch from a pinned fork" paragraph. | yes, first buildable commit |
| C4 `plugin: bar widget on the native app` | `plugin/oliver.torrents/` | yes |
| C5 `tests: native integration, engine benchmarks` | `agent_ergonomics_audit/audit/regression_tests/` (without `__pycache__`, and without `timer_wheel_tests.rs` and `run_timer_wheel_tests.py`, which P0-02 step 5 retires) | yes |
| C6 `docs: engine evaluation, queue and VPN plan, TUI review, validation receipts, 2026-10-18 plan` | `docs/` | yes |

   The source lands as one commit (C3) because intermediate slices would not compile: `main.rs`
   declares every module. The commit message lists the areas so the history stays readable.
4. **Prove completeness from a clean clone:** `git clone <repo> "$TMP/fbt" && cd "$TMP/fbt" &&
   cargo build --release --locked && agent_ergonomics_audit/audit/regression_tests/all.sh`. This
   catches any file that is still untracked but needed.
5. Push the branch, open a pull request "Land the native engine", merge with a merge commit (the
   slices stay visible), tag `v0.1.1`.

### 8.2 Vendored code

| Tree | Size | What it is | Decision | Why |
|---|---|---|---|---|
| `vendor/rqbit` | 3.3 MB | A maintained, deep fork: Asupersync runtime port, MSE, interface binding (`vendor/rqbit/README.md`) | Commit in-tree as workspace members (as `Cargo.toml:54-57` already declares) | The fork is part of the product; its changes must be reviewed with the app; it is small. |
| `vendor/asupersync` | 57 MB | crates.io Asupersync 0.5.0 (upstream rev `78b64636…`) plus about 690 changed lines: the timer-wheel cleanup in `src/time/wheel.rs`, the `epoch-diagnostics` feature and their tests (`vendor/asupersync/FBITTORRENT_PATCH.md`; `diff -r` against the registry copy) | **A pinned git fork:** a repository `github.com/odcpw/asupersync` whose branch `fbittorrent-0.5.0` is the upstream source plus one commit with the patch; `Cargo.toml` uses `[patch.crates-io] asupersync = { git = "…", rev = "<sha>" }` (today `path = "vendor/asupersync"`, `Cargo.toml:49-52`). Offer the wheel fix upstream. | 57 MB in every clone, forever, for a 690-line change is the wrong trade. A `rev` pin is as reproducible as a path. The patch note already asks to drop the patch once upstream has it. |
| `vendor/nova3` | 64 KB | qBittorrent's nova3 search runtime, embedded with `include_str!` (`src/search.rs:37-47`) | Commit in-tree after checking the licence headers of each file | Small and compiled into the binary; fbittorrent is MIT, so the embedded files' licences must allow it (unverified until P0-03). |

Decision (the owner, 2026-10-18, Q3): the public fork. Option B is the runnable fallback if the
repository is not created (P0-02's checkpoint): commit `vendor/asupersync/`
as it is, as its own commit before C3 (`src/` alone is 56 MB in 959 `.rs` files, measured with
`du` on 2026-10-18; `examples/`, `assets/` and `tests/` are under 100 KB together, so a
`cargo package`-shaped copy is not smaller and the choice is the fork or the full tree). A git
submodule is rejected: it complicates clones and builds
and offers nothing over a git dependency. With the fork, CI and `install.sh` need network for the
first fetch; `cargo build --locked --offline` works after one online build, and the README says so.

### 8.3 Validation receipts

- Q4, answered by the owner on 2026-10-18: "no archive; data deleted, README summaries kept in
  git".
- Keep in git: every `docs/validation/**/README.md` (the human-readable summaries), nothing else.
- Delete: everything else under `docs/validation/` (about 279 MB of data, scripts, logs and caches,
  including the four copies of IMDb's `title.ratings.tsv`); there is no archive. A README that links
  to deleted data keeps its link; the receipts are not rewritten.
- From now on `.gitignore` ignores `docs/validation/**` except `README.md` files and files
  explicitly added with `git add -f`. Receipts over 1 MB never enter git.

### 8.4 Continuous integration

`.github/workflows/ci.yml`, on push and pull request, with `rust-toolchain.toml` pinning 1.98.0
(the version `all.sh` already uses, `all.sh:6`):

| Job | Steps | Added by |
|---|---|---|
| `test` | `cargo test --locked -p fbittorrent -p rqbit-runtime -- --nocapture`, then the worker-panic scan from `all.sh` | P0-06 |
| `engine-libs` | the combined engine library command, which builds again after P0-05 | P0-06 |
| `native` | `cargo build --release --locked`; install `tmux` and `util-linux`; `native.py`; later `daemon.py --no-systemd` (P1-07), `agent_contract.py` and `scripts/verify-determinism.sh` (P1-11, it needs the built binary and a disposable daemon) and the native tmux smoke (P4-10); every suite uploads `target/test-logs/` as an artifact | P0-06, P1-07, P1-11, P4-10 |
| `lint` | `git diff --check`; the QML token lint (P2-05); the plugin structure lint (no `Timer`/`Process` outside `Service.qml`, P2-04); `scripts/lint-sizes.sh` (P1-02, P4-01) | P0-06, P1-02, P2-04, P2-05, P4-01 |
| `audit` (weekly schedule) | `cargo audit` over the lockfile (R8) | P0-06 |

- Local only, through `all.sh` flags: the Quickshell fixture and smoke tests (`FBT_TEST_QS=1`; they
  need Quickshell, Qt 6.12 and the Omarchy shell modules), the visual captures
  (`FBT_TEST_VISUAL=1`), `widget_keys.sh` (`FBT_TEST_WIDGET=1`), `vpn.py` namespace fixtures
  (`FBT_TEST_VPN=1`), and the resource gate (`FBT_TEST_RESOURCES=1`).
- **Resource gate:** `resources.py` runs before each release; it compares idle and active CPU and RSS
  with the baseline JSON from P1-08 and fails on a regression above the margin. The margin is set
  to three times the spread observed over three baseline runs, never below 10 %.
- `cargo fmt --check` is not added: the code base does not follow rustfmt today, and a format-only
  diff would bury the history of the uncommitted work.
- Unverified: whether GitHub Actions minutes are available for this repository. If not, `all.sh`
  is the gate and P0-06 documents running it before every merge.

### 8.5 Documentation

- `docs/IPC.md` and `contracts/ipc/v1/` (P1-05), with a changelog.
- README, rewritten in stages so it is never wrong for long: lifecycle and CLI (P1-11), install and
  plugin (P2-07), dashboard (P3-16), keys table generated from the action table (P4-02). The stale
  claims of D14 are corrected in P0-12 to describe today's behaviour, and restored in P4-07 when the
  code matches them.
- `robot-docs` and `capabilities` move to contract 1.2.0 at P1-11 (the 32 KiB/s cap, the daemon,
  `serve`, `restart`, `dashboard`, `--no-start`, the "reads never start, writes do" rule, the new
  `status` shape, a pointer to `docs/IPC.md`) and grow by patch versions as later items land
  (`log` at P1-14, `events` and `api` at P1-18, `--until playable` at P5-05, …), each with a
  `changes[]` entry; `commands[]` is generated from clap, so it never lists a verb before it exists.
  Why a minor bump: every change is additive for an agent that follows `robot-docs` (new verbs,
  new optional fields) except the `status` stopped shape, which is declared under
  `capabilities.changes[]` with `breaking: true` (Appendix F, Q12); a 2.0.0 would make agents
  re-read a contract that is otherwise unchanged.
- `docs/engine-evaluation.md` gains a section on fast resume when P6-06 lands.
- `CHANGELOG.md` starts at `v0.1.1`.

### 8.6 Release and install

- `scripts/install.sh` (P1-04; P2-07 adds `--plugin=link`): build with `--release --locked`; install the binary into
  `~/.local/bin` by copying to a temporary name and renaming, so a running daemon keeps its inode
  and detects the replacement (§3.2.12); then `fbittorrent setup --magnet --plugin=link`.
- `fbittorrent setup` becomes idempotent and gains `--dry-run`: config and download folder, the
  magnet handler and desktop entries with the new icon, the systemd unit plus `daemon-reload`, the
  plugin (`--plugin=link` symlinks the repository folder for development, `--plugin=copy` copies
  it for a stable install), `omarchy plugin enable oliver.torrents`, the bar placement only if it
  is missing, and the menu entries in `~/.config/omarchy/extensions/omarchy-menu.jsonc`:
  `torrents.open` summons the dashboard, `torrents.search` summons it on Search, `torrents.add`
  stays `fbittorrent add-clipboard`, `torrents.tui` keeps the terminal entry, and `torrents.stop`
  is gated on `fbittorrent status --quiet` instead of the stale
  `systemctl --user is-active qbittorrent-nox` check (`omarchy-menu.jsonc:36`, D18).
- `fbittorrent setup --uninstall` removes the unit, desktop entries, icon, plugin link and menu
  entries and keeps the data; `--purge` also deletes data after a confirm.
- Release checklist (not a ceremony; each line is a script): `all.sh` green; `FBT_TEST_QS=1`,
  `FBT_TEST_VISUAL=1` and `FBT_TEST_RESOURCES=1` runs green; the capture set reviewed; `CHANGELOG.md`
  entry; tag.
- Versions: the binary and the plugin share one SemVer number; the API protocol (`proto` 1) and the
  CLI contract (1.x) are versioned separately.

---
## 9. Work breakdown

### 9.1 Conventions

- **IDs** are `P<phase>-<nn>`; milestones are `M0` to `M6`. IDs are stable; new items get new numbers.
  Round 1 added P0-15, P0-16, P1-18, P2-08, P3-17, P3-18, P4-11, P5-05, P5-06, P6-13 and marked
  P7-07 superseded. Round 2 added P1-19 (split out of P1-02, which keeps its ID for the file move)
  and P5-07. Round 3 added the milestone M1a (the end of P1), marked P7-08 superseded, converted
  the P6 and P7 tables to the bullet form so every item carries Do, Why, Accept, Tests and
  Depends, and rewrote every item to be self-contained: files, API names, tokens, thresholds,
  fixture paths and section anchors inline (the preamble of §9.3).
- **Size:** S is under a day of agent work and roughly under 300 changed lines; M is one to three
  days or 300 to 1,000 lines; L is more than three days or more than 1,000 lines. Rough guides,
  not promises.
- **Every item** has: Do (what), Why (rationale), Accept (criteria a reviewer can check without
  reading the code), Tests (unit tests and end-to-end cases by name, and what they log), Depends.
  "Done" means the acceptance criteria pass, `all.sh` is green, CI is green (once P0-06 exists),
  and the relevant docs are updated in the same change.
- **Test logging contract (P0-15).** Every integration harness writes one JSON object per line to
  `target/test-logs/<suite>/<case>.jsonl`:
  `{"ts": "<iso8601>", "suite": "daemon", "case": "idle_exit_no_torrents", "step": 3,
    "action": "wait idle_exit_in_s == 0", "expect": "process exits 0 within 5 s",
    "observed": "exit 0 after 2.3 s", "ok": true, "ms": 2310}`.
  The daemon's stderr for that case goes to `target/test-logs/<suite>/<case>.daemon.log`; QML
  marker lines (`FBTQS-STEP {json}`) are collected into the same shape by the harness; there is no
  Rust twin, because every process-level case lives in `daemon.py`. A failure prints both file paths and their
  last 20 lines; `all.sh` ends with a table of suites, cases, passes, failures and the slowest
  five cases. `FBT_TEST_VERBOSE=1` mirrors every line to stderr as it is written. Unit tests run
  with `--nocapture`, and `all.sh` keeps failing the run on background worker panics (its existing
  scan, `all.sh:12-16`).
- **Fixtures:** local only. No test touches the owner's daemon, torrents, feeds or shows; every
  suite uses temporary XDG directories (the `native.py` model, `native.py:44-50`), local HTTP and
  peer fixtures, disposable info hashes, and `PATH` shims for `notify-send`, `xdg-open`,
  `wl-paste`, `systemctl` and `omarchy-launch-or-focus-tui` that record their argv.
- **Git safety:** no `git stash`, `checkout`, `reset` or `clean` until P0-04 is merged, and none on
  files an agent did not create afterwards.
- **No ceremony:** an item exists only if a user, an agent or a test consumes its output. Reports,
  reviews and checklists are not items; scripts are.
- **Receipts:** Appendix D defines the word; an item that names a receipt writes that section in the
  same change. A receipt records what was run and seen; it is not a report.

### 9.2 Phases at a glance

| Phase | Goal | Ends with | Release |
|---|---|---|---|
| P0 | The product is in git; test harness and goldens exist; live defects fixed on today's architecture | M0 "safe on main" | v0.1.1 (landing), v0.1.2 (fixes) |
| P1 | `fbittorrent serve`, systemd on demand, API v1 behind today's UIs, agent `events`/`api` | M1a "daemon behind today's UIs": the magnet click without a terminal, the TUI as a client | v0.2.0-pre (installed on the owner's machine, not tagged) |
| P2 | Bar popup on API v1, pushed, no polling, the §4 design language | **M1, first demo** | v0.2.0 |
| P3 | Search, RSS, Shows, IMDb in the daemon; dashboard window; poster grid; show quality | M2 "every step of W1, W2 and W8 works from the dashboard" | v0.3.0 |
| P4 | TUI as a pure client; module split; 17 UX fixes; legacy removed | M3 "TUI client" | v0.4.0 |
| P5 | T1 features: peers and seeds, recheck, turtle and global limits, ready to watch (with `stall_free`), low-disk warning | M4 | v0.5.0 |
| P6 | T2 features: trackers, pieces, graphs, stats, queue control, fast resume, undo, clipboard watch, … | M5 | v0.6.0 |
| P7 | T3 features | M6 | as they land |

P4-01, P4-05 and P4-06 can run in parallel with P2 and P3 once P1-15 is done; the DAG allows it.
Agents working in parallel take items from different lanes (§9.4) to avoid editing the same
files. The weighted critical path to M1 (32 S-units, §9.4) runs through P0-01 → P0-02 → P0-04 and
M0, then P1-01 → P1-02 → P1-19 → P1-05 → P1-06 → P1-08, and then through either of two branches of
equal weight: the daemon branch P1-12 → P1-17 → P1-11 → P1-15 → M1a, or the plugin branch
P2-01 → P2-02 → P2-03 → P2-05; P1-03
(L) and P1-07 (M) sit on a third parallel branch, so the daemon core and the typed operations can
be built by two agents at once (P1-03 depends on P1-02 and P1-07, not on P1-19). The plugin
lane can start `ApiClient.js`, `Store.js` and the P2-08 harness against P1-06's server and the
P1-05 fixtures before P1-08's bus exists (a first cut may poll `torrents.list` over `api.sock`
for a day and switch to `subscribe` when P1-08 lands), without changing P2-01's acceptance, which
needs the pushed path. M1a, the end of P1, is the first installable build (32 S-units, the same
weight as M1: after P1-08 the daemon branch P1-12 → P1-17 → P1-11 → P1-15 and the plugin
branch P2-01 → P2-02 → P2-03 → P2-05 both weigh 9, §9.4).

### 9.3 Items

Every item below is written to be handed to an agent alone: it names the files it touches, the API
messages and fields it uses (by their Appendix A names), the tokens, thresholds, formats, scripts
and fixture paths it relies on, and the section (with a row name where a table is meant) that holds
the canonical rule it implements. Where an item would otherwise copy a canonical section, it gives
the anchor plus the concrete parameters inline. Paths: `regression_tests/` is
`agent_ergonomics_audit/audit/regression_tests/`; `plugin/tests/` is the plugin test folder outside
the symlinked `plugin/oliver.torrents/` (§4.1); `contracts/` is at the repository root; test logs
go to `target/test-logs/<suite>/<case>.jsonl` in the shape of §9.1 and Appendix E.1.

Two rules apply to every item without being repeated in each. **Contract bookkeeping:** an item
that adds an API operation also adds its row in `api::ops::TABLE` (`mutating`, `idempotent` per
A.1, `since`; P1-18), a `docs/IPC.md` changelog entry (§3.4.4), its `caps` entry (§3.4.3) and a
`contracts/ipc/v1/<group>.ndjson` fixture with at least one `c2s`/`s2c` pair (§3.4.11); an item
that adds a CLI verb lists it in `capabilities.commands[]` (P1-11's `commands_match_clap` fails
otherwise), uses the `act()` JSON shape `{ok, action, count, hashes}` (`src/main.rs:718-728`)
unless Appendix F gives another, and maps errors through A.3; an item that adds a key regenerates
`keymap.json` and the README keys block with `scripts/gen-keymap.sh` (P4-02) and adds the Appendix
B row; a bulk verb expands `all` client-side and sends it in chunks of 256 hashes; every item
after P1-11 that adds a verb or a JSON field bumps the contract's patch version (1.2.1, 1.2.2, …)
and adds its `capabilities.changes[]` entry; `list`, `show`, `status` and `wait` keep the
qBittorrent-shaped torrent of contract 1.x and gain only fields with new names (P1-11). **Defaults for edge states:** `torrents.start`, `stop`, `set_seed`, `remove` and `add` are ledger
intents and work on every record (a `metadata` torrent can be paused, which stops the
resolution), with one carve-out: `start` on a completed stop-on-complete record is
`invalid_state` with the hint `fbittorrent seed <hash>` (P0-10, A.3) and `start all` lists it
under `skipped`; a mutation that needs a live engine handle (`recheck`, `reannounce`, `move`,
`set_files`, `set_upload_limit`, `queue_move`, `force_start`) on a torrent that has none (dormant,
staged, `metadata`, `done` without seeding) answers `invalid_state` unless the item says what it
does instead (`recheck`, and `set_files` with a non-empty `include`, restore a dormant torrent
first, P1-16); any operation that needs the engine answers `engine_unavailable` while the slot
is `Parked` (§3.2.9) and, while it is `Starting`, waits for the librqbit session for up to 60 s
before answering retryable `engine_unavailable` (§3.2.4: a magnet click right after `READY=1`
must not fail); a read on such a torrent answers what the ledger knows and omits the rest
(§3.4.9).

#### Phase P0: safeguard, harness and live fixes

**P0-01 Archive validation receipts out of the working tree** (S)
- *Do:* Back up the whole working tree as in §8.1 step 1 (a temporary copy; Q4, answered
  2026-10-18: "no archive; data deleted, README summaries kept in git"). Write
  `scripts/check-doc-links.sh` (the `scripts/` directory is new in this item): it lists every
  Markdown link from tracked `*.md` files into `docs/validation/`,
  resolves each against the tree, and writes one line per link to
  `target/test-logs/docs/links.jsonl` (`{ts, suite: "docs", case: "links", step, action: "<file>:<line> -> <target>", expect, observed, ok}`),
  exiting 1 when a link to a receipt `README.md` is broken; links to deleted receipt data are
  listed, not failed. Keep every `docs/validation/**/README.md` and delete everything else under
  `docs/validation/` (§8.3; no manifest, no archive). Write the receipt section `## P0-01` in
  `docs/validation/2026-10-18/README.md` (Appendix D "Receipt"): what was deleted and the
  link-check summary.
- *Why:* 279 MB of receipts (231 MB from one day) must not enter git history; they are evidence,
  not source [RC §12].
- *Accept:* `docs/validation/` holds only `README.md` files, none over 200 KB;
  `scripts/check-doc-links.sh` exits 0; the links from `docs/*.md` into `docs/validation/*/README.md`
  resolve; the worktree backup's SHA256 is in the receipt.
- *Tests:* `scripts/check-doc-links.sh` logs each link and its result to
  `target/test-logs/docs/links.jsonl` and exits non-zero on a broken link; it runs once here and by
  hand whenever receipts move (not a CI job: nothing under `docs/validation/` changes between
  releases).
- *Depends on:* none.

**P0-02 Host the Asupersync patch as a pinned git fork** (S)
- *Do:* Q3 and Q30 are answered (2026-10-18, "who cares, continue": the public fork, the rider's
  risk accepted by the owner, DL-70; `vendor/asupersync/LICENSE` is "MIT License (with
  OpenAI/Anthropic Rider)", `license = "LicenseRef-MIT-OpenAI-Anthropic-Rider"` in
  `vendor/asupersync/Cargo.toml:118`; every distribution must carry the rider unmodified, and the
  fork is a distribution). **Checkpoint before anything is pushed:** the **owner** creates the
  empty public repository `github.com/odcpw/asupersync` (creating or pushing a public repository
  is publishing; an agent never does it on its own) or says "fallback"; the agent asks once, in one
  message that offers both, and proceeds on either answer without asking again. On "fallback",
  skip steps (1) to (7) and run the option-B path at the end of this item. Otherwise, with the
  rustup toolchain (`~/.rustup/toolchains/1.98.0-x86_64-unknown-linux-gnu/bin/cargo`, as
  `all.sh:6`; the `cargo` on `PATH` is a mise shim with no version until P0-05): (1) import the
  crates.io package as the first commit (`~/.cargo/registry/src/*/asupersync-0.5.0`, from
  `~/.cargo/registry/cache/*/asupersync-0.5.0.crate`, sha256
  `f34b1a19ffd6b74570339a156912436335bb09c594c675c62ea164b19a1f2511`; exclude the registry marker
  `.cargo-ok`); (2) add one commit "fbittorrent patch" that is `vendor/asupersync/` copied over it
  (`cp -a`, then `git add -A`; the tree has no symlinks or empty directories). The patch is **not**
  only the timer wheel: `diff -r` against the registry copy shows 10 files, about 1,308 lines added
  and 11 removed: `src/time/wheel.rs` (+192, the `next_deadline` cleanup) and the restored upstream
  `src/time/wheel_metamorphic_tests.rs` (626 lines the crates.io package left out); the
  `epoch-diagnostics` feature (`Cargo.toml` +2, `src/runtime/epoch_diagnostics.rs` 196 lines,
  `src/runtime/epoch_tracker.rs` +113/-10, `src/runtime/mod.rs` +3, `src/runtime/builder.rs` +26);
  `RegionOwnerKey` and `Cx::reusable_region_owner_key()` in `src/cx/cx.rs` (+63, with
  `src/cx/region_owner_tests.rs`), which the rqbit fork calls (`vendor/rqbit/librqbit/src/spawn_utils.rs:63,86,96`)
  and without which the build fails; and an unconditional change in `src/runtime/state.rs`
  (`EpochConsistencyTracker::with_independent_module_sequences`, a `VecDeque` violations store).
  Rewrite `vendor/asupersync/FBITTORRENT_PATCH.md` to list exactly these (today it says the only
  implementation change is `wheel.rs`) and carry it in the fork commit. Branch `fbittorrent-0.5.0`
  is the default branch; tag the commit `fbittorrent-0.5.0-1` so the rev survives a branch
  rewrite. (3) In the fork's working copy run `cargo test --all-features` (the 63 timer-wheel tests
  of `docs/validation/2026-10-11/cpu-optimization/timer-wheel-tests/receipt.json` plus
  `region_owner_tests` green) before pushing. (4) Switch `Cargo.toml:49-52` from `path =
  "vendor/asupersync"` to `[patch.crates-io] asupersync = { git =
  "https://github.com/odcpw/asupersync", rev = "<40-hex sha>" }` with the comment "Asupersync 0.5.0
  plus the fbittorrent patch (FBITTORRENT_PATCH.md in the fork); drop when upstream has it"; run
  `cargo update -p asupersync` once (the `asupersync` entry at `Cargo.lock:218` has no `source`
  line today; afterwards it reads `git+https://github.com/odcpw/asupersync?rev=<sha>#<sha>`), then
  `cargo build --release --locked` and `cargo test --locked -p fbittorrent -p rqbit-runtime`. (5)
  Retire the timer-wheel runner instead of retargeting it: delete
  `regression_tests/timer_wheel_tests.rs` and `run_timer_wheel_tests.py` from the tree C5 commits
  (they hard-code the vendored path, `timer_wheel_tests.rs:3,65`, `run_timer_wheel_tests.py:55-57,112-113`,
  default `--support-lock` to a file that does not exist, `:83-84`, and can never pass from a
  clean clone because `select_core` accepts only a registry or vendored rlib). The patch's 63
  timer-wheel tests and `region_owner_tests` live in the fork and run there in step (3); the
  receipt records that run, and P0-01's archive keeps the receipt-era runner. (6) Archive the
  vendored tree: `mkdir -p ~/Archive/fbittorrent`; from the repository root `tar --zstd -cf
  ~/Archive/fbittorrent/asupersync-vendored-2026-10-18.tar.zst vendor/asupersync`; verify by
  listing and `sha256sum`; remove `vendor/asupersync`. (7) Add the §8.2 sentence to the README:
  `cargo build --locked --offline` works after one online build. (8) Write the receipt `## P0-02`
  (Appendix D) with the fork URL, tag and sha, the archive SHA256, the test counts, and the link to
  the upstream offer: an **issue** on `Dicklesworthstone/asupersync` describing the `wheel.rs` fix
  only (the region-owner key and the epoch changes are fbittorrent-specific; upstream `main` has
  moved past `78b64636…`), opened by the owner or with the owner's explicit permission.
  **Option B, the runnable fallback (the owner said "fallback"):** commit `vendor/asupersync/` as it
  is, with the `FBITTORRENT_PATCH.md` rewrite of step (2) and `LICENSE` byte-identical to the
  registry copy's, as commit C0 before C1 in the §8.1 table; leave `Cargo.toml:49-52` (`path =
  "vendor/asupersync"`) as it is; run the patch's tests in place (`cargo test --all-features
  --manifest-path vendor/asupersync/Cargo.toml`, the 63 timer-wheel tests and `region_owner_tests`)
  and record the counts; still delete the two runner files of step (5); skip the archive of step
  (6), since the tree stays in git; write the receipt with "option B" and the tree's SHA256 in
  place of the fork URL. Nothing is asked again. Accept under option B: `cargo build --release
  --locked --offline` passes from a clean clone with no network.
- *Why:* §8.2.
- *Accept:* `cargo build --release --locked` and `cargo test --locked -p fbittorrent -p rqbit-runtime`
  pass with the git dependency; `Cargo.lock` pins the revision; `diff -r --exclude=.git
  --exclude=.cargo-ok` between the fork checkout at the tag and the extracted archive is empty;
  `cargo test --all-features` in the fork checkout passes the 63 timer-wheel tests and
  `region_owner_tests` (recorded in the receipt); the two runner files are absent from C5.
- *Tests:* the build and test runs, logged by hand (P0-15's `fbtlog.py` does not exist yet) in the
  §9.1 / E.1 shape to `target/test-logs/asupersync/{build,test,fork_test}.jsonl` (one step per
  command with its exit code and duration, `final` lines; the worker-panic scan of `all.sh:12-16`
  applies).
- *Depends on:* P0-01 (the worktree backup must exist before step 6 removes `vendor/asupersync/`);
  the owner's one act before the landing commit of C3 is the checkpoint above: creating the
  repository or saying "fallback" (Q3 and Q30 were answered on 2026-10-18).

**P0-03 Repository hygiene** (S)
- *Do:* `.gitignore`: `__pycache__/`, `*.pyc`, the `docs/validation/**` rule of §8.3 (ignore all,
  then `!docs/validation/*/README.md`, `!docs/validation/MANIFEST.sha256`,
  `!docs/validation/ARCHIVE.md`), `target/test-logs/`, plus the existing `target/` and
  `agent_ergonomics_audit/.scratch/`. Read the licence header of every file in `vendor/nova3/` and
  record file, licence and copyright line in `vendor/nova3/README.md`; if a file's licence does not
  allow embedding in an MIT binary (`include_str!`, `src/search.rs:37-47`), stop and ask the owner.
  Add `LICENSE` at the repository root (MIT, the licence `plugin/oliver.torrents/manifest.json`
  already declares; today no root licence file exists) and a `NOTICE` line pointing at
  `vendor/rqbit/LICENSE` (Apache-2.0) and, under option B, `vendor/asupersync/LICENSE` with its
  rider; with the fork, the rider travels with the fork and the README names it.
  Scan the tree to be committed for secrets (`rg -n -i "api[_-]?key|token|password|BEGIN .*PRIVATE KEY"`)
  and review every hit; paste the reviewed output into the P0-04 receipt.
- *Why:* nothing junk or secret may enter the first commit of the product.
- *Accept:* `git status --porcelain` shows no `__pycache__`, no receipts outside the keep-set; the
  licence note exists; `LICENSE` and `NOTICE` exist at the repository root; the scan output is
  reviewed and saved in the P0-04 receipt.
- *Tests:* none beyond the scan (it is a one-time check; the `.gitignore` rules are exercised by
  every later commit).
- *Depends on:* none.

**P0-04 Land the native product on `main`** (M)
- *Do:* §8.1 steps 3 to 5: commits C1 to C6 in the order and with the contents of the §8.1 table
  (C1 `vendor/rqbit/`, C2 `vendor/nova3/`, C3 `src/` + `Cargo.toml` + `Cargo.lock` + `README.md` +
  `CHANGELOG.md`, C4 `plugin/oliver.torrents/`, C5 `regression_tests/` without `__pycache__`, C6
  `docs/`), the clean-clone build and `all.sh` (step 4), a pull request "Land the native engine",
  a merge commit, tag `v0.1.1`. Write the receipt `## P0-04` (clone log, `capabilities --json`
  diff, the P0-03 scan).
- *Why:* the total-loss risk [RC §12].
- *Accept:* `main` holds the native engine; a fresh clone builds and passes `all.sh` (90 tests and
  `native.py`); `capabilities --json` from the clean-clone build equals the installed binary's
  output (the reality check found them byte-identical for the working tree); the tag is pushed.
- *Tests:* clean-clone logs in `target/test-logs/land/clone.jsonl` (one line per step: clone, build,
  each suite, the capabilities diff).
- *Depends on:* P0-01, P0-02, P0-03.

**P0-05 Pin the toolchain and fix the multi-crate engine test build** (S)
- *Do:* Add `rust-toolchain.toml` (`channel = "1.98.0"`, the version `all.sh:6` already uses). Make
  `librqbit-core` resolve one SHA-1 backend when combined with `librqbit`: depend on it from
  `vendor/rqbit/librqbit/Cargo.toml` with `default-features = false, features = ["sha1-ring"]`, and
  check the other crates that enable `sha1-crypto-hash` by default
  (`vendor/rqbit/librqbit-core/Cargo.toml:30`). Update the commands in `docs/engine-evaluation.md`
  to the one that works: `cargo test --locked -p librqbit-core -p librqbit --features librqbit/rust-tls --lib`.
- *Why:* D11; the documented reproduction commands must work, and CI needs one command.
- *Accept:* that command builds and passes; the documented commands pass; at least 182 engine
  library tests pass in total.
- *Tests:* the commands, logged to `target/test-logs/engine-libs/run.jsonl` (one step per crate
  with the pass count).
- *Depends on:* P0-04.

**P0-06 CI workflow and gates** (M)
- *Do:* `.github/workflows/ci.yml` with the jobs of §8.4 (`test`, `engine-libs`, `native`, `lint`,
  `audit` weekly), each uploading `target/test-logs/` as an artifact; `all.sh` flags
  (`FBT_TEST_QS`, `FBT_TEST_VISUAL`, `FBT_TEST_WIDGET`, `FBT_TEST_VPN`, `FBT_TEST_RESOURCES`,
  `FBT_TEST_VERBOSE`, `FBT_TEST_FORCE_FAIL`) with a `--help` header that documents each; the
  resource gate scaffold: `resources.py --baseline regression_tests/baselines/resources.json`
  compares idle and active CPU and RSS with the baseline and fails above the margin of §8.4 (three
  times the spread of three baseline runs, never below 10 %), with a first baseline from today's
  `resources.py`; `all.sh` ends by running `scripts/testlog-summary.py` (P0-15). Commit
  `scripts/dagcheck.py` and `scripts/mockwidth.py` (Appendix E; review-round tools, not CI jobs).
- *Why:* the native engine has never had CI; every later phase relies on it.
- *Accept:* CI is green on `main`; a deliberately failing test on a throwaway branch turns it red
  and the uploaded `target/test-logs/` artifact contains the failing case's JSONL; `all.sh --help`
  documents the flags; `all.sh` ends with the summary table of §9.1.
- *Tests:* the workflow itself; `scripts/testlog-summary.py` has a unit test over a fixture
  `test-logs/` tree (`regression_tests/fixtures/testlogs/`: two suites, one failure, expected table
  text next to it).
- *Depends on:* P0-05, P0-15.

**P0-07 The widget never starts the app** (S)
- *Do:* Remove the `reconcile` timer and function (`plugin/oliver.torrents/Service.qml:78`,
  `121-126`). In native mode `reconcile` answers `{"ok": true, "touched": 0, "note": "native
  scheduling is intrinsic"}` without connecting (`src/main.rs`, the `Cmd::Reconcile` arm). Add the
  global flag `--no-start` and the variable `FBITTORRENT_NO_START=1`: `backend::connect()` then
  fails with exit 4 (`code_name: "backend"`, the existing dictionary `src/agent.rs:12-45`)
  instead of calling `ensure_running`. The plugin passes `--no-start` on every action
  (`Service.qml`'s argv builder). `Service.qml:15` becomes
  `readonly property bool online: status.ok === true && status.running !== false`, so the widget
  keeps hiding when `status` adopts the 1.2.0 stopped shape `{ok: true, running: false}` (DL-22)
  at M1a, weeks before P2-03 replaces it. Contract 1.1.1 (`src/agent.rs:134-210`) lists the flag under
  `global_flags[]`.
- *Why:* D1.
- *Accept:* re-running the probe of `evidence/reconcile-autolaunch-probe.txt`: `status`,
  `reconcile` and `--no-start list` never invoke the launcher; a widget action while the app is
  closed reports "fbittorrent is closed"; `Service.qml` tests `status.running !== false` in its
  `online` expression (a one-line `rg` in `all.sh` until P2-04's lint takes it over).
- *Tests:* unit: `Cmd::Reconcile` in native mode returns the stub without `connect`; e2e:
  `native.py::launcher_never_called` with a `PATH` shim for `omarchy-launch-or-focus-tui` that
  records its argv to a file, asserting zero calls across `status`, `reconcile`, `--no-start list`,
  and logging each invocation's argv and exit code as one step each.
- *Depends on:* P0-04, P0-15 (the `native.py` case structure the test is written in).

**P0-08 Graceful shutdown on terminal close** (S)
- *Do:* Make SIGHUP and SIGTERM in the TUI-hosted app drain the engine before exit. The mechanism is
  now grounded: ftui's signal thread records the signal, performs best-effort terminal cleanup and
  waits up to `SIGNAL_SHUTDOWN_GRACE = 2 s` for the runtime to clear
  `ftui_core::shutdown_signal::pending_termination_signal()`; if the runtime does not, it calls
  `exit(128 + sig)` (`ftui-core-0.6.0/src/terminal_session.rs:107,147-167,1270-1295`). The
  runtime's own event loop already polls the pending signal on every iteration
  (`ftui-runtime-0.6.0/src/program.rs:5254-5441`, through the wrappers at `:110-117`) and clears
  it only in `complete_lifecycle` **after** `shutdown_once` has restored the terminal
  (`:5462-5490`); the signal thread also accepts an explicit teardown as acknowledgement
  (`EXPLICIT_TEARDOWN_STARTED`, `terminal_session.rs:121,151,1078`). So the model needs no
  `Cmd::quit()` of its own; the 2.2 s are lost between the signal thread's record and that clear,
  most likely in `shutdown_once` writing to the dead pty, or a second SIGHUP arriving mid-drain.
  Steps: (1) with `FBT_DEBUG_SIGNALS=1`, append one line per event
  (`<ms since signal> <event>`) to `$XDG_STATE_HOME/fbittorrent/tui-signals.log` (stderr is the
  dead pty) for: signal recorded, loop observed it, `shutdown_once` begin/end, clear, `drop(app)`
  begin/end (`src/main.rs:819`); (2) if the clear happens after the grace, call
  `clear_pending_termination_signal()` from the model's `Msg::Hover` tick (`src/tui/mod.rs:1673`)
  as soon as it observes the signal and before returning `Cmd::quit()`, so the signal thread is
  acknowledged within 300 ms and the engine drop (≤ 4 s, `src/engine.rs:517-546`) runs to the
  end; (3) if the loop never observes the signal because the input reader blocks on the dead pty,
  spawn a watchdog thread on first sight of the pending signal that drains the engine directly
  through a shared `Arc<AppEngine>` handle (sets `stopping`, runs the session drain with its 4 s
  bound, unlinks the socket) and then clears the signal itself. Keep whichever of (2) and (3) the
  instrumentation shows is needed; the acceptance below is the same. In both, the engine drop
  logs `engine drained in <ms> ms` at info level (today `src/engine.rs:536-537` prints only on
  failure), which is the "drain message" the acceptance looks for. Expected exit status after a
  graceful signal exit stays `128 + sig` (the runtime's `SignalTerminationError` contract).
- *Why:* D2. P1 makes the TUI a client, but until then the owner closes the window every day.
- *Accept:* killing the tmux window (automated) or closing the Ghostty window (manual, once, logged
  in the receipt) leaves no `engine.sock`; `engine drained in` appears in `tui.stderr`; the
  instance lock (`flock` on `$XDG_RUNTIME_DIR/fbittorrent/engine.lock`, `src/ipc.rs:136-164`) is
  acquirable with `flock -n` within 6 s of `tmux kill-window` (2 s grace plus the 4 s drain bound).
- *Tests:* unit (wrapped in `ftui_core::shutdown_signal::with_test_signal_serialization`, because
  the pending signal is one process-wide value): record a pending signal, run one `Msg::Hover`
  update, assert it is cleared and the model returned `Cmd::quit()`; e2e:
  `native.py::terminal_close_is_graceful` (a case of the P0-15 harness) kills the tmux window with
  `tmux kill-window`, polls `engine.sock` and `flock -n engine.lock` every 100 ms, and logs the
  elapsed time, the exit status and the `tui.stderr` tail (`native.py:157`).
- *Depends on:* P0-04, P0-15.

**P0-09 One bad record no longer blocks startup** (M)
- *Do:* In `engine::start` (`src/engine.rs:453-466`) return as soon as the librqbit session exists
  and restore every record in a spawned task (records with a cached `.torrent` under the engine's
  torrent store, `src/engine.rs:164-168`, first; then magnet-only records with the existing 90 s
  metadata timeout); a failure marks the record `error` (reason `restore_failed`, with the
  message), keeps it and continues. `torrents/info` lists unrestored records from the ledger as
  `checkingDL` (today's qBittorrent-shaped string for `checking`, `src/engine.rs:1431`) or
  `metaDL` (for `metadata`; `src/qbt.rs:51` already treats it as active), as the parked path
  already does for its records (`src/engine.rs:1233-1355`). `torrents/start` on
  such a record retries the restore. (P1-03 reads the per-record outcome to drive the slot and
  `engine.changed`; §3.2.4 "The slot becomes `Running` as soon as the librqbit session exists".)
- *Why:* D4.
- *Accept:* with one valid record and one whose cached `.torrent` is missing and whose magnet
  cannot resolve, the app opens in under 10 s and lists both, the bad one with its message.
- *Tests:* unit: `engine::tests::bad_record_does_not_block` with a corrupted cache file and an
  unresolvable local magnet; e2e: `native.py::startup_survives_bad_record` logging open time and
  the listed states.
- *Depends on:* P0-04, P0-15 (the `native.py` case structure).

**P0-10 Resume on a finished torrent tells the truth** (S)
- *Do:* `torrents/start` (the legacy route, `src/engine.rs:586-651` match) on a finished
  stop-on-complete record (`completion_on != 0` and `ratio_limit == 0.0`, `src/engine.rs:90-92`)
  returns a typed refusal that the CLI prints as `{"ok": false, "code": 1, "code_name":
  "failure", "error": "already complete; seeding is off for this torrent", "hint": "fbittorrent
  seed <hash>"}` (the envelope of `src/agent.rs:106-116`; after P1-11 the same case is the API's
  `invalid_state`, which A.3 maps to exit 1);
  `start all` skips such torrents and lists them in `skipped: [{hash, reason: "complete_no_seed"}]`;
  the TUI footer shows `(U) Seed` instead of `(P) Resume` for them (`src/tui/mod.rs:2040`); the
  widget hides its start button for them (`Panel.qml`, the row action visibility).
- *Why:* D5.
- *Accept:* [RC §5] defect 3 no longer reproduces; the 80×24 footer golden shows `(U) Seed`.
- *Tests:* unit: engine test for the typed refusal and the `skipped` list; TUI interaction test
  for the footer label; golden (P0-16); e2e: `native.py::resume_done_refused` asserting exit 1
  with the hint in the JSON envelope.
- *Depends on:* P0-04, P0-16.

**P0-11 Truthful peer labels** (S)
- *Do:* TUI details say "N peers connected" instead of "0 seeds, N leechers"
  (`src/tui/mod.rs:2199`); the widget drops the seeds part (`plugin/oliver.torrents/Model.js:79`);
  browse rows show no seed count when the source gives none (`src/browse.rs`, the YTS mapping);
  `capabilities` documents `num_seeds` as "always 0 in native mode: unknown" (`src/agent.rs`, the
  field notes of 1.1.1).
- *Why:* D6, principle 4.
- *Accept:* re-taken captures of screens 01, 08, 10 and 14 (`screens/`) contain no "0 seeds" or
  "0s".
- *Tests:* TUI render tests and goldens; `plugin/tests/lint.sh` greps `Model.js` for `"s"` suffix
  construction (the rule lands with the lint in P2-04; until then the grep is a line in `all.sh`);
  e2e: `native.py::no_fake_seed_counts` greps the captured panes.
- *Depends on:* P0-04, P0-16, P0-15 (the `native.py` case structure).

**P0-12 Contract and README drift** (S)
- *Do:* `src/agent.rs:186,221` describe the native cap as "32 KiB/s while downloading (non-zero caps
  must be at least 16 KiB/s); stops when complete unless seeding is enabled". README: YTS loads in
  the app; 1337x stays browser-only (`README.md:219-224`); the UPLOADED column and dedupe claims
  describe today's behaviour (`README.md:61-66`).
- *Why:* D7, D14.
- *Accept:* a unit test derives the text from `NO_UPLOAD_LIMIT` (`src/qbt.rs:44`) so it cannot
  drift again; `robot-docs` and `capabilities` say 32 KiB/s.
- *Tests:* that unit test (`agent::tests::upload_cap_text_from_constant`);
  `agent_contract.py::capabilities_upload_text` (P1-11 adopts it; until then a `run.sh` case).
- *Depends on:* P0-04.

**P0-13 Destructive confirms default to No** (S)
- *Do:* The TUI's destructive prompts (remove, delete files, unfollow, remove feed) accept only
  `y`/`Y`; Enter, Esc and any other key cancel (`src/tui/mod.rs:1255-1262`); the label reads
  `y/N`; the footer shows `(y) Remove  (Esc) Cancel`. The widget's trash button and `d` open an
  inline confirm strip with focus on Cancel (§4.5 "Remove"; `Panel.qml`, the row's trash handler
  at `:451`).
- *Why:* D8; [RC §9] defects 5 and 15.
- *Accept:* Enter on "DELETE … AND ITS FILES?" cancels; every destructive prompt kind is covered.
- *Tests:* TUI interaction tests (one per prompt kind: Enter cancels, `y` confirms, Esc cancels);
  golden of the prompt footer; `regression_tests/widget_keys.sh::W-010` (opt-in,
  `FBT_TEST_WIDGET=1`) presses `d` then Enter and checks the torrent remains, logging the
  `cursor()` state before and after.
- *Depends on:* P0-04, P0-16.

**P0-14 Private store permissions** (S)
- *Do:* Create every store's temporary file with mode 0600 before renaming (`shows.json` today ends
  up 0644; `src/shows.rs:259-262`); check `rss.json`, `search.json`, `engine/*`,
  `legacy-migration.json` and `config.toml` (the writers in `src/rss.rs:93-100`, `src/search.rs`,
  `src/engine.rs` ledger save, `src/config.rs`).
- *Why:* D10.
- *Accept:* after a Shows save `stat -c %a shows.json` prints 600; a unit test per store asserts it.
- *Tests:* those unit tests; e2e: `native.py::store_modes` stats every file under the temporary
  data dir after the run and logs path and mode.
- *Depends on:* P0-04, P0-15 (the `native.py` case structure).

**P0-15 Test logging contract and shared harness module** (S)
- *Do:* `regression_tests/fbtlog.py`: `Case(suite, name)` context
  manager that writes the JSONL lines of §9.1 and Appendix E.1 to
  `<repo>/target/test-logs/<suite>/<case>.jsonl` (`<repo>` is the parent of
  `agent_ergonomics_audit/`, never the current directory; the suite directory is emptied when the
  suite starts). `step(action, expect)` returns a step handle whose `ok(observed)` or
  `fail(observed)` closes it; `ms` is the wall time between `step()` and the close; `step`
  numbers from 1 per case; `ts` is UTC ISO-8601 with milliseconds and a `Z`. A failing step marks
  the case failed, is logged with `ok: false`, and the case continues to its end (so one failure
  shows every later symptom); the suite exits non-zero if any case failed. Case slugs are
  snake_case of today's `ok:` labels (`dec_mouse_mode`, `closed_status_stays_closed`,
  `one_enter_adds_once`, `refresh_preserves_read_history`, `rss_mouse_and_scoped_shortcuts`,
  `mouse_selection_at_scroll_offset`, `native_cli_fields_and_deletion_safety`,
  `shows_inbox_and_targets`, `file_modal_shows_help_80x24`, `settings_queue_vpn_restart`,
  `quit_restart_down`; `native.py:172-376`). `Case` captures a child process's stderr into one
  suite-wide file and records the byte offset at each case boundary, so `<case>.daemon.log` is the
  slice for that case (`native.py` has one `tui.stderr` for the whole run, `native.py:157`; the
  slice is what gets printed on failure). Only the capture a step asserts on is logged; polling
  captures inside `wait_for` are logged only under `FBT_TEST_VERBOSE=1`, which also mirrors every
  line to stderr (both are part of this item). `FBT_TEST_FORCE_FAIL=<suite>/<case>` makes that
  case's first step fail, for the acceptance check and for `fbtlog_test.py`. The `qs` shim is
  `plugin/tests/qs-log.py` (outside the symlinked plugin folder, §4.16): it runs `qs -p <dir>`
  with the given arguments and `NO_COLOR=1`, reads stdout and stderr merged (`qs` logs on stderr
  with a `DEBUG qml:` prefix), turns each `FBTQS-STEP {json}` marker found anywhere in a line
  (same field names as §9.1)
  into a JSONL line, turns `FBTQS-PASS <case>` / `FBTQS-FAIL <case> <diff>` into the case's
  `final` line, and exits 1 on any FAIL; it is built here and first used by P2-02. There is no
  Rust twin of this module: every process-level case lives in `daemon.py` or `native.py` (§9.1),
  and Rust unit tests log through `cargo test`'s own output. Convert `native.py` to `fbtlog.py`
  (its 11 `ok:` prints become the cases above; `native.py --case <slug>` and `--list` select and
  list them, the same switches `daemon.py` gets in P1-07). `scripts/testlog-summary.py` reads
  `target/test-logs/`, prints the table of §9.1 (suite, cases, passes, failures, slowest five) and
  exits 1 when any case has `ok: false`; `all.sh` calls it last, in this item (P0-06 adds the CI
  upload).
- *Why:* principle 10; the user asked for logging that lets a reviewer see what every test did.
- *Accept:* `native.py` produces `target/test-logs/native/<case>.jsonl` for each of the 11 cases
  with at least one line per CLI call or asserted pane capture; `FBT_TEST_FORCE_FAIL=native/quit_restart_down`
  prints both file paths and their last 20 lines and makes `all.sh` exit non-zero; the summary
  table lists 11 cases.
- *Tests:* `regression_tests/fbtlog_test.py` (plain `python3`, called by `all.sh`): a fake suite
  with one passing and one forced-failing case writes the expected lines (`step` 1..n, `final`
  verdicts, `ms` ≥ 0) and `testlog-summary.py` counts one failure and exits 1;
  `plugin/tests/qs-log.py --self-test` feeds itself three canned `FBTQS-*` lines.
- *Depends on:* P0-04.

**P0-16 Golden-render harness for the TUI** (S)
- *Do:* `src/tui/tests/golden.rs` and `src/tui/tests/fixture.rs`: a fixture `TuiApp` state (three
  torrents in the states `downloading`, `done` and `metadata` with the names of the §5.6 mockups,
  two followed shows with two unread releases, one feed with three items, ten search results
  including one mojibake name) rendered into a headless ftui frame at 80×24, 120×30 and 160×45
  for each screen (Torrents, Search, Shows, RSS) and modal (files, providers, settings, prompt,
  help), compared with `src/tui/tests/golden/<screen>-<w>x<h>.txt`; `FBT_UPDATE_GOLDEN=1`
  rewrites them; a failure prints a unified diff. Create the goldens from the unsplit code so
  every later change shows up as a diff. The golden text is the frame's cells joined by lines, no
  colour (colour is tested by `palette::tests`, P4-05).
- *Why:* P0-10, P0-11, P0-13 and the whole of P4 change renders; without goldens a reviewer cannot
  see what changed.
- *Accept:* 3 sizes × (4 screens + files, providers, settings, prompt, help) goldens exist and
  pass; changing one character of a label fails the test with a readable diff.
- *Tests:* itself; a meta-test asserts the fixture renders without panics at 60×20.
- *Depends on:* P0-04.

**M0 Safe on main.** *Depends on:* P0-06, P0-07, P0-08, P0-09, P0-10, P0-11, P0-12, P0-13, P0-14,
P0-15, P0-16. Tag `v0.1.2`; install it.

#### Phase P1: the daemon and API v1 behind today's UIs

**P1-01 A lifecycle object replaces the process-global statics** (M)
- *Do:* Create `src/daemon/mod.rs` (declaring only `pub mod lifecycle;` until P1-03 fills the rest)
  and `src/daemon/lifecycle.rs`: `Lifecycle` (an `Arc<(Mutex<State>, Condvar)>`) with
  `request_stop(StopReason)`, `request_restart(RestartReason)`, `wait(Duration) -> Option<Request>`
  (returns the oldest pending request or `None` on timeout), `expected_network() -> Option<NetworkState>`
  and `set_expected_network(..)`. `StopReason::{Signal(i32), Idle, Explicit, Panic, Upgrade, Quit}`
  carries its exit code (`Signal`, `Idle`, `Explicit`, `Quit` → 0; `Panic` → 70; `Upgrade` → 75;
  §3.2.6, §3.2.12); `RestartReason::{Settings, Network, Explicit}`. Replace `QUIT_REQUESTED`
  (`src/engine.rs:32`), `RESTART`, `CLOSE` and `EXPECTED` (`src/network_app.rs:9-11`) with a
  `Lifecycle` passed to the engine, `network_app` and the TUI. Behaviour does not change.
- *Why:* statics prevent a daemon that restarts engines in place and prevent tests with two engines
  in one process [RC §10].
- *Accept:* no lifecycle `static` remains (`rg -n "static .*(QUIT_REQUESTED|RESTART|CLOSE|EXPECTED)" src`
  is empty); all tests and `native.py` are green; a new test runs two engines with separate
  temporary directories in one process.
- *Tests:* `engine::tests::two_engines_one_process`; `lifecycle::tests::{stop_reason_wins,
  restart_then_stop}`; the existing suites.
- *Depends on:* M0.

**P1-02 Engine split (isomorphic)** (M)
- *Do:* Split `src/engine.rs` isomorphically into `src/engine/{mod,ledger,queue,ops,view,parked,legacy_import}.rs`
  as §3.3 names them (`mod.rs`: `AppEngine` start and drop; `ledger.rs`: `Record`, read, save,
  fsync; `queue.rs`: `reconcile_queue` and limits; `ops.rs`: today's string dispatch, which P1-19
  replaces; `view.rs`: today's `torrents/info` JSON builder, which P1-19 generalises; `parked.rs`:
  `ParkedLedger` from `dispatch_parked_in`; `legacy_import.rs`: `migrate_legacy`). Moves only; no
  behaviour change (the discipline P4-01 applies to the TUI). Create `scripts/lint-sizes.sh`: fails
  when any file under the directories it is given exceeds 900 lines (`src/engine/` here; P4-01
  adds `src/tui/`).
- *Why:* the typed operations and the normalized model (P1-19) need seams that a 2,694-line file
  does not have [RC §10]; keeping the move separate is what makes its isomorphism test meaningful.
- *Accept:* `cargo test`, `native.py` and the `torrents/info` JSON of a fixture ledger are
  byte-identical before and after; no file in `src/engine/` exceeds 900 lines.
- *Tests:* the isomorphism test (`engine::tests::info_json_identical_after_split`, run against a
  JSON captured before the move into `src/engine/tests/info-before-split.json` and logging the
  first differing JSON pointer); `native.py`; `scripts/lint-sizes.sh src/engine`.
- *Depends on:* P1-01.

**P1-19 Typed operations and the normalized model** (M)
- *Do:* In `src/engine/ops.rs` add `enum EngineOp` with one variant per route of today's match
  (`src/engine.rs:586-651`: `torrents/info`, `torrents/files`, `torrents/add`, `torrents/start`,
  `torrents/stop`, `torrents/delete`, `torrents/filePrio`, `torrents/setUploadLimit`,
  `torrents/setShareLimits`, `app/version`, `app/shutdown`, `app/restart`, the `controls` routes
  of `src/network_app.rs:78-90`) and `EngineDispatcher::execute(op) -> EngineResult`; the string
  routes become one parser `EngineOp::parse(route, body)` used by the legacy socket (P1-10). In
  `src/engine/view.rs` add `TorrentView` with the fields of A.9 `Torrent` that exist today (the
  "Source today" column of A.10) and the normalized `state`/`reason` derivation of §3.5, plus the
  single qBittorrent mapping `TorrentView::to_legacy_json()` for old clients (the strings of
  `src/engine.rs:1418-1448`). Remove the duplicated route lists (`src/engine.rs:549-571`,
  `586-651`, `1233-1246`; `src/network_app.rs:78-90`). Add `Ledger.generation: AtomicU64`, bumped
  on every ledger save, paired with a `std::sync::Condvar`, so the event bus thread of P1-08 can
  wait for ledger changes without locking the async `Mutex<Ledger>` (`src/engine.rs:154`) from a
  std thread (§3.4.7).
- *Why:* API v1 needs typed operations; one table ends the four-place route sync [RC §10].
- *Accept:* each route name appears once; old clients behave identically (`native.py`, all TUI
  tests); every combination of rqbit state and record flags maps to exactly one `state`.
- *Tests:* `view::tests::state_mapping_table` (table-driven over every `TorrentStatsState` ×
  `paused` × `finished` × `stop_on_complete` × `record.paused` combination, logging the row that
  fails); `ops::tests::every_route_named_once` (greps the route parser table against the legacy
  socket's list); `ledger::tests::generation_bumps_on_save` (one save, one bump, one `Condvar`
  wake); `native.py`.
- *Depends on:* P1-02.

**P1-03 Daemon core: `fbittorrent serve`** (L)
- *Do:* `src/daemon/{mod,slot,sdnotify}.rs` and the `serve` subcommand in `src/main.rs`
  (§3.2.1: flags `--idle-exit-secs N`, `--no-notify`). `run_serve()`
  in order: validate the runtime directory (§3.2.5: absolute, owned by the uid, 0700, not a
  symlink; else exit 1); take `engine.lock` with `flock(LOCK_EX|LOCK_NB)` retried for 5 s
  (`src/ipc.rs:136-164`; a holder means `already_running`, exit 1, hint `fbittorrent status --json`);
  set `SIGPIPE` to `SIG_IGN` and block `SIGTERM`, `SIGINT`, `SIGHUP` before any thread, with one
  `sigwait` thread calling `lifecycle.request_stop(StopReason::Signal(n))` (§3.2.6); install the
  panic hook (log, `request_stop(StopReason::Panic)`, exit 70); unlink and bind `engine.sock`
  (served by today's string dispatch through `EngineOp::parse` only after P1-10; until then the
  existing `src/ipc.rs` server loop moved under the daemon); create the slot
  `Arc<RwLock<Slot>>` with `Slot::{Starting, Running(AppEngine), Parked(ParkedLedger), Stopping}`
  and `engine_epoch: u64` (§3.2.8); send `READY=1` to `$NOTIFY_SOCKET` as soon as the socket
  listens and the slot is `Starting` (§3.2.4; `src/daemon/sdnotify.rs` is std-only: one
  `UnixDatagram` with `READY=1`, `STATUS=<text>`, `STOPPING=1`); start the engine with P0-09's
  background restores and move the slot to `Running` when the session exists; or, with binding
  configured and `network_app::state(cfg).ready == false` (`src/network_app.rs:22-36`), start
  `Parked` (§3.2.9). The supervisor thread (§3.2.8): every 1 s or on a lifecycle signal: stop →
  leave; restart-class setting → `restart_engine(Settings)`; network change while binding is
  configured → park or unpark with a new `engine_epoch`; every 30 s the upgrade check (P1-17 fills
  it). Shutdown (§3.2.7 "any → Draining"): slot := `Stopping`; drop the engine (session ≤ 4 s,
  runtime ≤ 4 s, `src/engine.rs:517-546`); unlink sockets; release the lock; `STOPPING=1`; exit
  with the reason's code. Logging per §3.2.15 (`level source message key=value…` on stderr).
  Messages name the configured interface, not "ProtonVPN" (`src/engine.rs:831`, `1248`).
- *Why:* the enabling step (§0.1).
- *Accept:* in an isolated XDG directory `serve` starts and sends `READY=1` (to a fake
  `NOTIFY_SOCKET`) before any hash check finishes; with `network_interface = "fbmissing0"` it
  starts parked; it exits 0 on SIGTERM within 10 s with the drain messages `engine drained in <ms>
  ms` and `sockets unlinked`, and 70 after an injected panic (`FBT_TEST_PANIC_AFTER_READY=1`); the
  lock is free afterwards; a second `serve` exits 1 with `already_running`.
- *Tests:* `daemon.py::{ready_notify, parked_start, sigterm_drains, panic_exits_70,
  second_serve_refused}` (the first daemon cases of the P1-07 skeleton, which spawns the binary with
  a fake `NOTIFY_SOCKET` and reads the datagrams); the unpark case runs with the namespace fixture
  from `vpn.py` (`vpn.py::headless_park_unpark`, opt-in `FBT_TEST_VPN=1`).
- *Depends on:* P1-02 (the split engine; today's string dispatch serves `engine.sock` until the
  legacy adapter lands), P1-07 (the harness skeleton).

**P1-04 systemd user unit and start on demand** (M)
- *Do:* `src/setup.rs` writes `~/.config/systemd/user/fbittorrent.service` with the text of §3.2.2
  (the `ExecStart` path is `current_exe()`), runs `systemctl --user daemon-reload`, and `setup
  --uninstall` removes it. Create `src/api/client.rs` with `ensure_daemon()` implementing §3.2.4
  steps 1 to 5 (its liveness probe is `app/version` on `engine.sock` until P1-11 switches it to
  `hello` on `api.sock`; the 200 ms connect timeout, the 30 s `systemctl --user --no-ask-password
  start fbittorrent.service` bound, the `deactivating` wait of step 3, the exit-4 hint of step 4
  when there is no user manager (§3.2.14) and the hint of step 5). New
  subcommands in `src/main.rs`: `up [--ui]` (start only; `--ui` also opens the dashboard or the
  TUI), `down` (`app/shutdown` over `engine.sock` until P1-11), `restart` (P1-17 fills it),
  `dashboard` (runs `omarchy-shell shell summon oliver.torrents '<payload>'` when the plugin
  manifest has a `panel` kind, else `tui`; Appendix F). `backend::connect()` goes through
  `ensure_daemon()`; `--no-start` and `autostart = false` follow §3.2.3 (exit 4, hint
  `fbittorrent up`). The magnet handler desktop entry (`src/setup.rs:111-114`, `Exec=fbittorrent
  add %u`) no longer opens a terminal; `fbittorrent.desktop` runs `fbittorrent dashboard`. Create
  `scripts/install.sh` here (§8.6; P2-07 only extends it): `cargo build --release --locked`, copy
  the binary to `~/.local/bin/fbittorrent.tmp`, `mv` it over `~/.local/bin/fbittorrent` (a plain
  `cp` onto a running binary fails with `ETXTBSY`), then `fbittorrent setup --magnet`, so the
  M1a build is installed the same way as every later one and `setup` has run before the first
  magnet click. `FBITTORRENT_UNIT` (default `fbittorrent.service`) names the unit
  `ensure_daemon()` starts and `setup` writes, so a test harness with a throwaway name never
  touches the owner's unit.
- *Why:* D12 and the lifetime goals.
- *Accept:* `systemctl --user enable fbittorrent` creates no symlink under
  `~/.config/systemd/user/*.wants/` (static unit); `fbittorrent add <magnet>`
  with nothing running starts the unit and adds without any launcher call; `status` leaves the unit
  inactive; `down` stops it; after `kill -9` systemd restarts it within 3 s;
  `systemctl --user show fbittorrent -p KillMode` prints `mixed`; ten `start`/`down` cycles in 30 s
  leave the unit startable.
- *Tests:* `daemon.py::{unit_is_static, add_starts_unit_no_launcher, status_never_starts,
  down_stops, kill9_restarts, killmode_mixed, start_limit_not_hit, start_while_deactivating
  (the §3.2.4 step-3 retry)}` (real user systemd locally with a throwaway unit name derived from
  the temp dir; the no-manager path with `daemon.py --no-systemd`, which runs `serve` as a
  foreground child, in CI); each case logs `systemctl`
  outputs and the launcher shim's call count.
- *Depends on:* P1-03.

**P1-05 API v1 contract: types, IPC.md, fixtures, contract tests** (M)
- *Do:* `src/api/mod.rs`: serde types for Appendix A, P1 scope first (A.1 envelope with `instance`,
  A.2 `hello`, A.3 `Error` and every code, the A.4/A.5 operations with `Since` P1-xx, the A.7
  topics `summary`, `torrents`, `torrent:<h>.files`, `network`, `settings`, `events`, the A.8
  events, the A.9 objects `Torrent`, `TorrentDetail`, `File`, `Summary`, `Limits`,
  `NetworkState`, `Settings`, `LogEntry`), all `Option<T>` fields with
  `#[serde(skip_serializing_if = "Option::is_none")]` (§3.4.9) and `#[serde(deny_unknown_fields)]`
  on `args` structs (§3.4.4). `docs/IPC.md`: framing (§3.4.1), envelope (§3.4.2), handshake and
  caps (§3.4.3), versioning (§3.4.4), error codes (A.3), the idempotent list (A.1), limits
  (§3.4.10), and a changelog starting at "v1 (P1-05)". Fixtures
  `contracts/ipc/v1/{hello,errors,torrents,subscribe,subscribe-gap,mutations}.ndjson` in the
  §3.4.11 line format (`c2s`, `s2c`, `expect`, `volatile`), `subscribe-gap.ndjson` feeding a
  skipped `rev`, `hello.ndjson` also carrying an `s2c` `hello` with `proto: 2` for the clients'
  protocol-mismatch banners (§3.2.1; the daemon has no flag to fake its version), and
  `errors.ndjson` carrying the request-less `error` event followed by `bye` (A.1, A.8). Contract
  test 1 (round-trip) and the sentinel scan (§3.4.9).
- *Why:* principle 6; Sprout's fixture discipline [OR §8 item 1].
- *Accept:* every P1-scope operation and event has a type and at least one fixture; the tests run in
  `cargo test`.
- *Tests:* `api::fixtures::{roundtrip, no_sentinels}` (`roundtrip` deserializes every fixture line
  into the typed enums and re-serializes to canonical JSON, so an unknown `dir` or a malformed
  frame fails there with its line number; `no_sentinels` walks every fixture object for `-1`,
  `8640000` and zeros in "absent when unknown" fields; no separate schema script).
- *Depends on:* P1-19.

**P1-06 API v1 server** (L)
- *Do:* `src/api/server.rs` and `src/api/ops/{app,transfer,torrents}.rs` (§3.4.1 to §3.4.6,
  §3.4.10, §3.9; the wire types and envelope of A.1, the error event of A.8, the `state`/`reason`
  derivation of §3.5 through P1-19's `TorrentView`): the listener on `api.sock` in the runtime directory, bound under the lock
  (§3.2.5); `SO_PEERCRED` uid check on accept (log and close a foreign uid); the first frame must
  be `hello` within 5 s (`hello_required`); `hello` answers A.2 with `caps` (`torrents`, `files`
  now; later items append theirs) and `limits {line_in: 65536, line_out: 1048576, inflight: 8,
  subs: 16, hashes: 256}`; line caps (64 KiB in, 1 MiB out; over → `invalid_message` and close);
  the dispatch pool (8 threads, queue 64, then
  `busy`); per-connection reader and writer threads with a 512-frame outgoing queue (`bye
  {reason: slow_client}` on overflow, §3.4.7); at most 8 in-flight ids per connection (the 9th is
  `busy`); op timeouts 15 s default and 95 s for `torrents.add` (`timeout`, outcome unknown);
  operations `hello`, `app.info`, `app.client`, `app.shutdown`, `app.restart`,
  `app.network`, `transfer.info`, `torrents.list`, `torrents.get`, `torrents.add`,
  `torrents.start`, `torrents.stop`, `torrents.remove`, `torrents.files`, `torrents.set_files`,
  `torrents.set_seed`, `torrents.set_upload_limit`, `torrents.open_target` with the args and
  results of A.4/A.5 (including `File.sort_key` as A.9 defines it and `Torrent.uploaded_bytes`),
  each executing through `EngineOp` (P1-19) and answering `{state: "done"}`
  after the ledger write and the queue reconcile (§3.4.8); typed errors (A.3) with `hint` and
  `retryable`; `app.client {interactive, glance}` recorded per connection and reset on drop;
  `app.info.clients` counters per `client.kind`; the quiesce order of §3.2.7 on shutdown (stop
  accepting, answer in-flight requests, `bye {reason}` last, close).
  Rules the tests above assume: the 5 s `hello` deadline counts from `accept` and is not reset by
  bytes without a newline; at the deadline the server sends `error {hello_required}`, `bye` and
  closes; a second `hello` is `invalid_state`; `invalid_message` (not JSON, not an object, over
  the cap, invalid UTF-8; an empty line too, while a `\r\n`-terminated line is accepted because
  the `\r` is JSON whitespace) always closes the connection, `invalid_args` (`id` 0, negative,
  non-integer or reused; an unknown top-level key; duplicate keys, which serde alone resolves by
  keeping the last, so the server deserializes each frame through a `Visitor` over `MapAccess`
  that errors on a repeated key) answers with `"id": null` and keeps the connection; a timed-out op releases its in-flight slot at the timeout and its late
  result is dropped; `busy.details.limit` is one of `connections`, `inflight`, `dispatch`,
  `sessions`, `subs`; `unknown_op.details.nearest` lists names within Levenshtein 2 (P1-18);
  arguments that later items add (`stage`, `restore`, `force`, `origin` values other than
  `manual|handler|agent`) are `invalid_args` until those items land, and `ref` is accepted and
  ignored until P1-09; `app.network.interfaces` is the `settings --interfaces` listing
  (`network_app::interfaces()`, `src/main.rs:391`); `upgrade_pending` is `false` until P1-17.
- *Why:* the contract every client will use.
- *Accept:* contract test 2 replays the fixtures against a live daemon; the 33rd connection gets
  `busy`; a 70 KiB line gets `invalid_message` and the connection closes; a non-object line is
  refused; the peer-credential check refuses a foreign uid (unit test of the check with injected
  credentials); `hello` after 5 s of silence gets `hello_required`.
- *Tests:* `daemon.py::contract_{hello,errors,torrents}` (each logs the JSON pointer of any
  mismatch); `daemon.py::{connection_limit, line_cap, hello_deadline, non_object_line,
  hello_twice, ninth_inflight_busy, slow_hello_bytes}`; `api::server::tests::peercred_rejects_other_uid`;
  `api::server::tests::garbage_table` (table-driven: invalid UTF-8, empty line, 65,537
  bytes, 10,000-deep nesting, duplicate keys, `id: 0`, string `id`, unknown top-level key; each
  must answer `invalid_message`/`invalid_args` and never panic; a `\r\n`-terminated `hello` must
  be accepted; and the table logs the input's
  description next to the answer). The NDJSON client helper of `daemon.py` (`ApiConn.hello()`,
  `.call(op, args, ref=None)`, `.events()`) lands here.
- *Depends on:* P1-03, P1-05, P1-19 (`EngineOp` and `TorrentView`, which every operation here runs
  through; stated directly rather than through P1-05).

**P1-07 Integration harness `daemon.py`** (M)
- *Do:* `regression_tests/daemon.py` on `fbtlog.py`, as a skeleton that needs no daemon to exist:
  temporary XDG directories (`XDG_RUNTIME_DIR`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`,
  `XDG_STATE_HOME`, `XDG_CACHE_HOME` under one `mkdtemp`), `regression_tests/fixtures.py` (a
  local HTTP fixture server serving a directory; the local magnet fixture from `native.py`:
  a 256 KiB random payload, its `.torrent` with a local HTTP tracker, and its magnet;
  `mkv_fixture(64 MiB)` with a minimal Matroska header carrying a duration, `iso_fixture`,
  `two_file_fixture` whose second file starts mid-piece, `corrupt_byte(path, offset)`; canned
  servers `eztv_server()`, `imdb_server()`, `torznab_server()` reading JSON from
  `regression_tests/fixtures/<name>/`; `path_shim(name, script)` writing an executable that
  appends its argv to `<tmp>/shims/<name>.log` and prints a scripted stdout), a process spawner
  that captures stderr into `<case>.daemon.log`, the case runner (`daemon.py --case name`,
  `--list`) and the `--systemd` / `--no-systemd` switch (a throwaway unit name derived from the
  temp dir). Later items add the helpers they need and their cases: P1-03 the first daemon cases,
  P1-06 the NDJSON client helper, P1-07's own second half is `--serve-fixture <state>` (a
  daemon left running with fixture torrents in named states for the visual harness, P2-08; the
  state names are the rows of `plugin/tests/VISUAL.md` and each is a JSON recipe under
  `regression_tests/fixtures/states/<state>.json` listing torrents to add and the manipulations
  to apply: pause, throttle, corrupt, fill the disk) and the local seed/leech peer pair (a
  second `fbittorrent serve` in another temp dir seeding the fixture, with `--throttle <bps>` for
  P5-05 implemented as that daemon's `transfer.set_limits {upload_bps}`), which land as a
  follow-up commit once P1-06 exists.
- *Why:* one harness for every daemon behaviour, runnable in CI.
- *Accept:* runs locally and in CI (`--no-systemd`) in under 3 minutes; a failing case prints its
  log paths; `--serve-fixture downloading` leaves a daemon with one downloading torrent until
  killed.
- *Tests:* the harness is covered by every case that uses it; `fbtlog_test.py` (P0-15) covers the
  logging shape and the failure tail. No `--self-test` mode.
- *Depends on:* P0-15 (the skeleton; the peer-pair and `--serve-fixture` half lands as a follow-up
  commit once the API server exists, see Do).

**P1-08 Event bus: subscriptions, snapshots, deltas** (L)
- *Do:* `src/api/bus.rs` (§3.4.7): `subscribe {topics, interval_ms}` and `unsubscribe {sub}` (A.6);
  topics `summary`, `torrents`, `torrent:<h>.files`, `network`, `settings`, `events` (A.7, with
  the `seq` ring of 1,000 entries or 10 minutes and `since_seq` replay, `snapshot.begin {reason:
  "resync"}` when too old); chunked snapshots (`snapshot.begin`, `snapshot.chunk` ≤ 256 items,
  `snapshot.end`, A.8) taken under the ledger lock with the atomicity rule; `delta {changed,
  added, removed}` with `null` for "now absent"; `state` frames for unkeyed topics; immediate
  structural frames driven by a fork accessor `ManagedTorrent::state_changed(&self) -> impl Future`
  (added to `vendor/rqbit/librqbit/src/torrent_state/mod.rs`; it selects over the existing
  `state_change_notify` at `:212` **and** the live state's `finished_notify`
  (`torrent_state/live/mod.rs:201,910`), because completion fires only the latter; both are
  `Notify` with `notify_waiters`, so the bus registers `notified()` before it samples the state and
  re-samples after every wake, which also covers `:333`, where the notify precedes the `Error`
  assignment; a fork unit test asserts it wakes on pause, unpause, completion and error) and by the
  ledger's change counter (P1-19 adds `Ledger.generation: AtomicU64`, bumped on every save, and a
  `std::sync::Condvar` the bus thread waits on; the bus reads torrent stats through the engine's
  `block_on` path like dispatch does, `src/engine.rs:577-583`, never by locking the async
  `Mutex<Ledger>` of `src/engine.rs:154` from a std thread); rate and progress changes coalesced to `interval_ms` (clamped
  250..10000, per-topic defaults of A.7 when omitted, A.6); `rev` increasing by one per frame per
  subscription; `heartbeat {ts}` after 15 s of silence; closing slow clients with `bye
  {reason: slow_client}` when the 512-frame queue is full; `engine.changed {epoch, state,
  reason}` followed by a fresh snapshot with `reason: "engine_restart"` on torrent-like
  subscriptions; detail-topic sampling released when the last subscriber leaves;
  `app.info.subscriptions {total, by_topic}` (A.4) so tests can see a detail topic released;
  `subscribe` and `caps` gain `subscribe`. Measure the budget (§3.10) with `resources.py --daemon` for 0, 1 and 8
  subscribers and 10 and 100 torrents; write `docs/validation/<date>/daemon-budget.md` and the
  baseline `regression_tests/baselines/resources.json` that P0-06's gate reads.
  Spelled out: `resources.py --daemon --torrents N --subscribers K --seconds 60` (extending the
  existing `regression_tests/resources.py`, which measures the TUI today) starts a disposable
  daemon with N local fixture torrents (the P1-07 magnet fixture duplicated with distinct
  payloads, paused, so the CPU is the bus's, not a transfer's), opens K `summary` subscriptions
  at 5,000 ms, and records user+system CPU seconds from `/proc/<pid>/stat` and peak RSS from
  `/proc/<pid>/status` (`VmHWM`) over 60 s; the reference is `fbittorrent status --json` run
  every 10 s for 60 s against the same daemon, measured the same way on the CLI processes;
  `resources.json` is `{"configs": {"<torrents>x<subscribers>": {"cpu_s": [3 runs], "rss_kb":
  [3 runs]}}, "reference": {"cpu_s": [3 runs], "rss_kb": [3 runs]}}` (P0-06's gate reads the
  spread of the three). "On change" topics (`network`, `settings`, `events`, `log`, the RSS and
  Shows topics) ignore `interval_ms`. After `engine.changed`, every `torrents`, `summary` and
  `torrent:<h>.*` subscription gets a fresh snapshot; a `torrent:<h>.*` topic whose hash no
  longer exists gets `snapshot.begin {count: 0, reason: engine_restart}` and `snapshot.end`, and
  the client closes it. The slow-client log line is `warn api slow client kind=<kind>
  name=<name> queued=512` (§3.2.15).
- *Why:* pushed updates replace per-tick polling everywhere.
- *Accept:* `subscribe-torrents.ndjson` replays; a client that stops reading is closed when its queue
  fills while other clients keep receiving; `rev` increases by one per frame in a 10-minute soak;
  with 100 fixture torrents and 8 subscribers the numbers are recorded and the bar-subscription
  criterion of §3.10 holds (one `summary` subscription at 5,000 ms costs no more CPU than today's
  `status --json` every 10 s); a client fed a gap resubscribes (fixture `subscribe-gap.ndjson`).
- *Tests:* unit: `bus::tests::{diff_changed_added_removed, coalesce_rates_not_states, rev_monotonic,
  queue_full_marks_slow, events_ring_resync}`; `daemon.py::{slow_client_closed_others_live,
  engine_restart_keeps_connection, rev_soak_10min (opt-in, FBT_TEST_SOAK=1), detail_topic_released,
  structural_frame_within_100ms (pause a torrent; the `delta` with `state: paused` arrives within
  100 ms at a 10,000 ms interval)}`; `resources.py --daemon` for the budget, logging CPU and RSS
  per configuration.
- *Depends on:* P1-06, P1-07.

**P1-09 Mutations: refs, pending, confirmed** (M)
- *Do:* §3.4.8 in `src/api/mutations.rs` (add it to the §3.3 layout): the ref ring, 256 entries
  per `client.name`, each entry keyed by `(client.name, ref)`, oldest evicted first, every entry
  expiring 10 minutes after it was stored (`Ring::with_clock(fn() -> Instant)` so
  `ring_expiry` can advance time); an evicted or expired ref is a fresh request. A ref seen again
  with a different `op` or different `args` is `invalid_args` with `details.key = "ref"` and the
  message "ref reused for a different request". The stored response is whatever was answered,
  including `pending` and errors (a client that wants to retry a failed mutation picks a new
  ref); a replay while the first call is still pending returns the stored `pending` response and
  the later `mutation.result` reaches the replaying connection too. Server-generated refs are
  `srv-<first 8 hex of instance>-<counter>`. `done` and `pending`; `mutation.result {ref, op,
  outcome, error?, result?}` goes to the connection that sent the request, to any connection that
  has since replayed the same `(client.name, ref)`, and to the `events` topic; `outcome: timeout`
  fires when the operation's own timeout passes (§3.4.10: 95 s for `torrents.add`, 15 s default;
  `recheck`, `remove` with files and `move` have no timeout and always end `done` or `failed`).
  For `torrents.add`, `result` is `{hash, name, duplicate?}`; a URL add answers `{state:
  "pending", ref}` with no `hash`. Requests may carry `instance: str?` (A.1); a mutation whose
  `instance` is not the current one is applied as a fresh request when the op is idempotent (the
  idempotent list of A.1, the only place it is written) and refused with `invalid_state`
  ("daemon restarted since this request; check the list") for every operation A.1 lists as not
  idempotent (`torrents.remove`, `torrents.recheck`, `torrents.move`, `torrents.queue_move`, …);
  this is the mechanism behind §3.2.11. Mandatory
  `delete_files`. Every `hashes: [hash]` argument in A.5 is bounded 1..256; an empty list or
  more than 256 is `invalid_args` with `details.key = "hashes"`; `limits.hashes` in `hello` says
  256. `torrents.add` returns the hash at once for magnets (a base32 `btih` is decoded to 40
  lowercase hex; `btmh` v2 magnets are `unsupported`) and `.torrent` paths; `{url}` sources are
  fetched by the daemon with the §3.4.1 file rules (16 MiB, bencoded, 95 s) and a URL that
  resolves to a hash already in the ledger answers `duplicate: true` through `mutation.result`.
  Duplicate adds return the existing hash with `duplicate: true` and change nothing. A magnet add
  creates the ledger record at once from the hash and the `dn=` name (today the record is written
  only after metadata arrives, `src/engine.rs:905-950`) and the row is visible as `state: metadata`
  while the engine resolves it in the background (the P0-09 task pattern; the §4.3.1 "Fetching
  metadata · 0:12" row and P7-09 rely on this); `stop` on it saves the pause intent and cancels
  the resolution. `caps` gains `mutations`.
- *Why:* no optimistic lies, no double removes, no hash-diff race in the CLI.
- *Accept:* repeating a `torrents.remove` ref returns the first result and removes nothing more;
  two clients using the same ref string do not collide; `torrents.remove` without `delete_files` is
  `invalid_args`; adding a URL gives `pending` and then `mutation.result` with the hash; a reused
  ref with different args is `invalid_args`; a `remove` carrying a stale `instance` is
  `invalid_state` while a `stop` with the same stale `instance` applies; a duplicate add is `ok`
  with `duplicate`. (The CLI's list-diff removal, `src/main.rs:525-532`, `550-560`, is P1-11's
  acceptance, because the CLI moves to `api::Client` there.)
- *Tests:* `mutations.ndjson`; `daemon.py::{ref_replay_remove, ref_per_client, remove_requires_flag,
  add_url_pending_then_result, duplicate_add, ref_reused_different_args, stale_instance_remove_refused,
  stale_instance_stop_applies, hashes_cap_257}`; unit `mutations::tests::{ring_expiry, ring_evicts_oldest,
  replay_returns_stored_error}`.
- *Depends on:* P1-06, P1-07.

**P1-10 Legacy socket adapter** (S)
- *Do:* `src/api/legacy.rs`: the daemon serves `engine.sock` with the 4-byte length-prefixed JSON
  framing of today's `src/ipc.rs` (`ipc::request`'s format); each request's `route` and body parse
  into `EngineOp` through `EngineOp::parse` (P1-19); responses use the qBittorrent-shaped JSON of
  `TorrentView::to_legacy_json()` (`src/engine/view.rs`); `app/shutdown` calls
  `lifecycle.request_stop(StopReason::Quit)` and `app/restart` restarts the engine in place
  (§3.2.8). The adapter is removed in P4-09 (§3.6, v0.4.0).
- *Why:* the TUI keeps `qbt::Client` and the 4-byte framing until P4-03 (§5.2 step 1), so the
  daemon must answer that framing; it is the adapter's only consumer. One binary serves daemon, CLI
  and widget, so no v0.1 CLI or widget survives `install.sh`, and the v0.1 TUI cannot attach to any
  engine but its own (`src/main.rs:787,804-806`).
- *Accept:* every `qbt::Client` call the TUI makes (`torrents/info`, `files`, `add`, `start`,
  `stop`, `delete`, `filePrio`, `setUploadLimit`, `setShareLimits`, `app/*`) answers from the
  daemon's `engine.sock` with the qBittorrent-shaped JSON of `src/engine/view.rs`; `app/shutdown`
  stops the daemon and `app/restart` restarts the engine.
- *Tests:* unit `legacy::tests::route_parse_roundtrip` (every route above parses and re-serialises
  through `EngineOp`); `native.py`'s 11 cases run against the TUI-as-client in P1-15 and cover
  every route above.
- *Depends on:* P1-03, P1-19 (the route parser targets `EngineOp`).

**P1-11 The CLI on the API client; contract 1.2.0** (L)
- *Do:* **Client** (`src/api/client.rs`, created by P1-04): the blocking NDJSON client: connect with
  a 200 ms timeout (a non-blocking `connect` plus `poll`, since std has no Unix-socket connect
  timeout), `hello` with `client.name = "cli-<pid>"`, `kind: "cli"` (`"agent"` under `--json`,
  §3.4.3, so `app.info.clients` tells agent use from human use); a reader thread that
  matches responses to requests by `id` (at most 8 in flight, a channel per request) and queues
  unsolicited events separately; a `subscribe` iterator that yields frames and tracks `rev`
  (tested against the `subscribe*.ndjson` fixtures and, once P1-08 is in, the live bus); the
  reconnect backoff of §3.2.11; the mid-request rule of §3.2.4 step 6 with the idempotent set
  hard-coded from A.1 until P1-18's `api::ops::TABLE` replaces it; `ensure_daemon()`'s liveness
  probe becomes `hello` on `api.sock` first, then `engine.sock` (§3.6: a TUI-hosted v0.1 engine
  answers reads through `torrents/info` and refuses writes with "an older fbittorrent app is
  running; quit it with Q and retry"; checked by hand in the M1a receipt); `ensure_daemon()` reads
  `FBITTORRENT_UNIT` (set by `setup`, default `fbittorrent.service`; the harnesses set a throwaway
  name, P1-04) so a test never touches the owner's unit. The TUI reuses the client from P1-15
  (which depends on this item) and `events` from P1-18. **Scope:** the verbs that exist at P1
  (today's `Cmd` enum, `src/main.rs:48-234`, plus P1-04's `up`, `down`, `restart`, `dashboard`
  and P1-03's `serve`): every Appendix F row whose verb exists now moves to the client with the
  "Starts" rule of §3.2.3; rows for later verbs (`undo`, `recheck`, `reannounce`, `queue`,
  `limits`, `limit`, `move`, `rename`, `shows quality`, `api`, `events`, `log`, `wait --until`,
  `list --state/--origin`) land with their items. Reads fall back to the ledger with a top-level
  `running: false` (`list`, `show`, `triage`), exit 0, with or without `--no-start` (the flag and
  `autostart = false` only stop a command that would otherwise start the daemon, §3.2.3). `stop`
  and keep-files `delete` write the ledger offline in the lock order of §3.2.3 (at most two
  lock/hello rounds, then exit 4 with the daemon's hint; a `hello` that answers `stopping` waits
  for `deactivating` to end as §3.2.4 step 3; an offline delete also removes the record's
  `<hash>.torrent` as the parked path does, `src/engine.rs:1338-1340`; the JSON carries `running:
  false`). `settings` keeps today's `app/setQueue` and `app/restart` through the compat adapter
  until P3-05 moves it; `rss grab` and `shows grab` call `torrents.add` until P3-07 moves them to
  `rss.grab`/`shows.grab`; `search`, `imdb`, `rss *`, `shows *` stay in-process until P3-07. In
  legacy mode (`engine = "qbittorrent"`, until P4-09) the WebUI path stays and every command prints
  one stderr line "engine = \"qbittorrent\" is deprecated and is removed in v0.4.0; see the README"
  plus a `warnings[]` entry. **Shapes stay contract 1.x:** `list`, `show`, `status` and `wait`
  keep printing the qBittorrent-shaped torrent (`qbt::Torrent`, `src/qbt.rs:11-36`), rebuilt from
  the A.9 `Torrent` by `cli::shapes::torrent()` (`src/cli/shapes.rs`, the twin of
  `TorrentView::to_legacy_json()`, P1-19); the additions are new fields with new names:
  `native_state` (the §3.5 enum), `reason`, `checked`, `origin`, `eta_s` (absent when unknown) and
  `seeds` (P5-01), later `playable`; `state` keeps its qBittorrent string, and the old numeric
  fields keep qBittorrent's own sentinels so a 1.x parser never meets a missing key: `eta` is
  `8640000` when `eta_s` is absent (qBittorrent's documented "infinity"), `num_seeds` stays `0`
  until P5-01 fills it and `num_leechs` is the peer count, `progress_known: false` with
  `progress: 0` when `progress` is absent (the parked shape today); `capabilities.changes[]`
  (1.2.0) says so under `field: "list[].eta"` and `field: "list[].num_seeds"`, and `robot-docs`
  tells agents to read `eta_s`/`seeds`/`native_state`. `warnings: [{code, message}]` is added at the top level of every JSON
  envelope when non-empty (version skew, §3.2.12; the deprecation). The add path uses the hash
  returned by `torrents.add` and no longer diffs lists (`src/main.rs:525-532`, `550-560`); `add
  --origin manual|handler|agent` (default `manual`; the desktop entry runs `fbittorrent add --origin
  handler %u`; `robot-docs` tells agents to pass `--origin agent`) feeds `torrents.add {origin}`,
  which P1-06 accepts and stores from the start (the `origin` cap of P6-08 covers the filter and
  `{restore}`); the `add --json` shape is today's (`{ok, action: "add", hash, name, source,
  sequential, first_last}`) plus `duplicate` and later `deferred`. `all` expands client-side and is
  sent in chunks of 256 for every bulk verb; `stop all` with nothing to stop prints `count: 0`
  without calling. `wait`: one `ensure_daemon()` reconnect after `bye {reason: restart|upgrade}` or
  EOF; `bye {idle}` ends with today's "every waited torrent is stopped" envelope and exit 8
  (`src/main.rs:761-763`); `bye {slow_client|hello_required|invalid_message}` exit 1. **`status`**
  (§3.2.13): running → `{ok: true, running: true, connection: "api", backend: "native", app_open:
  <interactive count > 0>, daemon: {state, idle_exit_in_s?, busy_reasons, upgrade_pending}}` plus
  every 1.x running field unchanged (`engine`, `dl`, `up`, `active`, `queued`, `seeding`, `stopped`,
  `total`, `torrents: [<cli::shapes::torrent()>]` with the active ones first then up to 12 others,
  `src/main.rs:416-430`), because the v0.1 popup reads them until P2-03 (`Model.js:102-115`)
  from `hello`, `transfer.info` and `summary` (one `subscribe summary`, first `state` frame);
  stopped → `{ok: true, running: false, app_open: false, backend: "stopped", engine: <config>,
  total: N}` from the ledger; `status --quiet` prints nothing and exits 0 when running, 4 when
  stopped (the menu gate of §8.6, which only tests non-zero). **Contract 1.2.0** (`src/agent.rs`):
  `contract_version: "1.2.0"`; `commands[]` generated at runtime from clap's command tree (nested
  verbs as `rss list`, `shows follow`, …) with `CommandDoc` keeping `needs_backend` and gaining
  `starts: bool` (the §3.2.3 column) and `since: str`; `changes: [{since, breaking, command?,
  field?, note}]` with the `status.ok` break and every addition; new text for `engines`,
  `conventions.native_connect` (the daemon, `ensure_daemon`, the start rule), `defaults` and
  `files.socket` (`api.sock`); the §8.5 list as far as it exists at P1 (later items bump the
  patch version, 1.2.1 at P1-18 for `events`/`api`, and so on, each adding its `changes[]` entry);
  `robot-docs` rewritten for the daemon. **Tests and scripts:** `regression_tests/agent_contract.py`
  on `fbtlog.py` (the successor of `run.sh`: R-001 to R-012 ported against a disposable daemon
  started as a foreground child in the temp XDG directory, `--systemd` locally with the throwaway
  unit);
  `scripts/verify-determinism.sh` (starts a disposable daemon, runs `status`, `list`, `show <h>`,
  `triage`, `config`, `capabilities`, `robot-docs --json` twice with `SOURCE_DATE_EPOCH` set and
  diffs; when `SOURCE_DATE_EPOCH` is set the CLI uses it for every wall-clock-derived field it
  prints (`elapsed_secs` in `wait`, `generated_at` in `capabilities`), ledger `*_at`/`*_on` values
  are untouched; it runs in the `native` CI job, which has a build, §8.4). README: a new
  "Lifecycle" section before `## CLI` (from §3.2: the daemon, `up`/`down`/`restart`, the start
  rule, idle exit) and the CLI section updated.
- *Why:* W4 and W5; the agent surface stays stable and additive.
- *Accept:* `agent_contract.py` passes against the daemon; `capabilities --json` lists the 1.2.0
  changes and `contract_version == "1.2.0"`; read commands never start the daemon and answer
  `running: false` with exit 0 when it is down, with or without `--no-start`; every `--json`
  output is byte-identical across two runs with `SOURCE_DATE_EPOCH` set and the daemon idle;
  nothing but JSON appears on stdout in `--json` mode; `list --json` entries equal
  `cli::shapes::torrent()` of the A.9 objects `api torrents.list` returns (checked when P1-18
  lands).
- *Tests:* `agent_contract.py::{R-001…R-012 (ported), exit_code_dictionary_total (1: `start` on
  the completed local fixture; 2: a bad flag; 3: `delete --files` without `--yes`; 4: `--no-start
  add` with the daemon down, and `busy` through 33 held connections; 5: `imdb --refresh` with
  `FBT_IMDB_BASE_URL` at a closed port; 6: `show zzzz`; 7: an ambiguous selector; 8: `wait
  --timeout 1`), reads_never_start, reads_fall_back_offline, json_determinism,
  stdout_stderr_split, status_stopped_shape, status_running_shape, version_skew_warning (two
  builds through `FBT_BUILD_ID_OVERRIDE`, P1-17), capabilities_upload_text, commands_match_help
  (every subcommand `fbittorrent --help` lists is in `commands[]` and vice versa),
  deprecation_warning_legacy_mode, add_origin_flag, stop_all_empty_count_0,
  wait_survives_upgrade_restart, wait_exits_8_when_daemon_idles_out}` (the two `wait` cases cover
  an exit-75 restart mid-wait, which `wait` must survive with one reconnect, and a torrent paused
  by someone else so that the daemon idles out, which `wait` must report as the stopped case with
  exit 8 rather than reconnect forever); unit `api::errors::tests::retryable_never_maps_to_1`
  (over the A.3 table) and `agent::tests::commands_match_clap` (the generated list equals the
  `Cmd` tree); `daemon.py::{offline_stop_ledger, offline_delete_keep_files_ledger,
  offline_write_waits_for_draining_daemon}` (the last starts `down` and an offline `stop`
  together and asserts afterwards that `torrents.json` holds the torrent paused and no record was
  lost to the daemon's final save).
- *Depends on:* P1-04, P1-09, P1-12, P1-14, P1-17.

**P1-12 Stop rule and idle exit** (M)
- *Do:* `src/daemon/policy.rs` (§3.2.10): the one config key `idle_exit_secs` (default 300,
  minimum 30, `0` disables the idle net; `serve --idle-exit-secs` overrides), evaluated by the
  supervisor every 5 s; the busy reasons `downloading`, `seeding`, `parked_waiting`,
  `interactive`, `search`, `mutation` as the table of §3.2.10 defines them (`clipboard_watch` and
  `glance` are never busy); interactive counting through `hello {interactive}` and `app.client
  {interactive}` (the bar connection is never interactive, §4.4 rule 8); the glance pause
  (`app.client {glance}` freezes the countdown and releases it on `false` or disconnect);
  `summary.daemon.idle_exit_in_s` (the frozen value while a glance is open) and
  `summary.daemon.busy_reasons` (A.9); at zero `bye {reason: "idle"}` and exit 0. The owner's stop
  (`app.shutdown {reason: "user"}` from the popup's power button, `fbittorrent down` or the TUI's
  `Q`, P1-03's drain) is never gated by the busy table. There is no `stop_policy` key, no `last-ui`
  branch and no `explicit` value: the owner's rule of A1 replaced them (DL-70).
- *Why:* A1 (the owner's answer of 2026-10-18).
- *Accept (each a `daemon.py` case with `idle_exit_secs = 2`):* exits with no torrents; stays while a
  local download runs; stays while an interactive client is connected; exits with only a bar
  client; stays while parked with a waiting download; with `idle_exit_secs = 0` never exits on its
  own and still drains on `app.shutdown`; `busy_reasons` names the reason in every "stays" case.
- *Tests:* `daemon.py::idle_exit_{no_torrents, download_keeps_alive, interactive_keeps_alive,
  bar_does_not, parked_keeps_alive, disabled_never_exits, disabled_shutdown_drains, glance_pauses
  (open a glance at 240 s remaining, wait 30 s, close, assert `idle_exit_in_s` resumes at 240 and
  the exit happens 240 s after the close, not 210; this one case runs with `idle_exit_secs = 30`,
  so read "20 s remaining, wait 5 s, resumes at 20, exits 20 s after the close, not 15"),
  glance_drop_resumes (kill the glancing client; the countdown resumes)}`;
  unit `policy::tests::{countdown_cancel, countdown_pause_resume, zero_disables}`.
- *Depends on:* P1-08.

**P1-13 Daemon notifications and the icon** (S)
- *Do:* `src/daemon/notify.rs` (§3.8): a queue drained by at most 4 threads, `(kind, hash)`
  collapsed while pending, each running `notify-send -a fbittorrent -i fbittorrent -A
  <action>=<label>… "<title>" "<body>"` and reading the chosen action name from stdout; the rows
  "added" (title "Added", actions `undo` → `torrents.remove {delete_files: false}` and `window` →
  `fbittorrent dashboard`), "add failed" (`log` → `fbittorrent log`), "completed" ("Download
  complete", `default` and `open` → `xdg-open <open_target>`, `folder` → `xdg-open <dir>`), "VPN
  waiting / back" (no actions, once per transition); bodies escaped for `&` and `<`; `notify =
  false` (config) and `serve --no-notify` disable everything. Who posts "Added": the daemon, with
  its two actions, only when `torrents.add` carries `origin: "handler"` (the desktop entry; the
  `undo` action must outlive the short-lived `add` process); the CLI keeps today's plain
  `notify-send` (no actions, no `--wait`, `src/main.rs:1296-1300`) only on a TTY or with
  `--notify`, and never for `--origin handler`, so a click produces exactly one toast. The SVG
  icon `assets/fbittorrent.svg` (monochrome, the `󰇚` shape) installed by `setup` to `~/.local/share/icons/hicolor/scalable/apps/fbittorrent.svg`; replace
  `-i qbittorrent` (`src/main.rs:1296-1300`) and `Icon=qbittorrent` in the desktop entries
  (`src/setup.rs:112-114`) with `fbittorrent`.
- *Why:* T1 notifications; A5; the borrowed icon [RC §9].
- *Accept:* a fake `notify-send` records the calls with the right actions; the "open" action
  triggers a fake `xdg-open` with the right path; "undo" removes the just-added torrent keeping
  files; `notify = false` suppresses everything; a VPN flap of three transitions in 2 s produces at
  most two notifications.
- *Tests:* `daemon.py::{notify_completed, notify_action_open, notify_added_undo, notify_off,
  notify_vpn_once_per_transition, notify_queue_bounded (40 completions at once: at most 4
  `notify-send` processes alive, all 40 recorded)}`; unit `notify::tests::escape_markup`.
- *Depends on:* P1-08.

**P1-14 Daemon log ring, `fbittorrent log`, usage counters** (S)
- *Do:* `src/daemon/log.rs` (add it to the §3.3 layout): the ring of 2,000 `LogEntry` (A.9:
  `{seq, ts, level, source, hash?, message}`, sources per §3.2.15) fed by the stderr logger, with
  the journald copy unchanged; `app.log {since_seq?, limit? ≤ 1000, level?, source?, hash?}` →
  `{entries, next_seq}` (A.4); the `log` topic (A.7, keyed by `seq`, append-only); `fbittorrent log
  [--follow] [--json] [--level error|warn|info|debug] [--source <s>]` (Appendix F: exit 4 when down;
  `--follow` prints NDJSON); `app.info.clients` counters per `client.kind` in memory; retire
  `engine/last-error.txt` (`src/engine.rs:484-487`); `caps` gains `log`.
- *Why:* T1 log; D13's "feedback is unlogged".
- *Accept:* an injected engine error (`FBT_TEST_INJECT_ERROR=<hash>`) appears once in `fbittorrent
  log --json` with `source: "engine"` and its hash; counters increase per `hello` kind; `log
  --follow` prints a new entry within 1 s.
- *Tests:* unit `log::tests::{ring_bound, seq_monotonic, filter_level_source}`;
  `daemon.py::{log_injected_error, log_follow, hello_counters}`.
- *Depends on:* P1-08.

**P1-15 The TUI attaches to the daemon; quit verbs** (M)
- *Do:* §5.2 step 1: `fbittorrent tui` calls `ensure_daemon()` and runs against the daemon through
  `engine.sock` with `qbt::Client`'s native path and the 2 s poll unchanged (`src/tui/mod.rs:1670-1674`);
  the in-process lifecycle checks (`src/tui/mod.rs:1699-1702`) become "daemon gone" detection
  (two failed polls show the banner "fbittorrent stopped" with Enter → `ensure_daemon()`); `q` and
  Ctrl+C close the TUI only; `Q` sends `app.shutdown {reason: "user"}` after "Stop fbittorrent? N
  downloads pause until it starts again. y/N" when transfers are active; `q` never asks, because
  closing the TUI never stops a transfer (A1); the first `q` ever shows the one-time notice of §5.2
  stored as `{"first_quit_seen":
  true}` in `~/.local/state/fbittorrent/notices.json`. The TUI also opens an `api.sock` presence
  connection (`hello` with `client.name = "tui-<pid>"`, `kind: "tui"`, `interactive: true`,
  heartbeats; `app.shutdown` for `Q`) while its data still flows over `engine.sock`, so the idle
  net counts it as interactive. A second `fbittorrent tui` in another terminal attaches (today a second
  launch only focuses the first window, `src/main.rs:787`).
- *Why:* D9 and A1; attach over SSH (W6).
- *Accept:* starting the TUI with no daemon starts it; `q` leaves a local download running; `Q`
  with an active download asks and stops on `y`; the first `q` shows the notice once; killing the
  daemon shows "fbittorrent stopped" in the TUI within 5 s; a second TUI in another tmux pane
  attaches and both show the same torrent.
- *Tests:* `native.py::{tui_starts_daemon, q_detaches, Q_confirms, first_q_notice_once,
  daemon_gone_banner, second_tui_attaches}` (from this item `native.py` starts a disposable daemon
  in its temp XDG directory and the TUI attaches to it); TUI interaction tests for the prompts.
- *Depends on:* P1-04, P1-10, P1-11 (the `api::Client` the presence connection uses), P1-12.

**P1-16 Lazy restore of completed torrents** (M)
- *Do:* In `src/engine/mod.rs`'s restore task (P0-09), records with `completion_on != 0` that stop
  on complete (`ratio_limit == 0.0`) are restored dormant: served from the ledger and the saved
  `.torrent` as `state: done, checked: false` (A.9) without opening files or adding them to the
  librqbit session. `torrents.start`, `torrents.set_seed {on: true}`, `torrents.recheck`,
  `torrents.set_files` with a non-empty `include` and `torrents.open_target` on a dormant record
  trigger the real restore with a check first; a
  dormant record whose files are missing then becomes `error` with `reason: missing_files`.
- *Why:* D3; under A1 the daemon starts often.
- *Accept:* starting with five completed fixture torrents performs no piece checks for them (no
  initial-validation log lines) and lists them as done within 1 s; `seed` on one checks it and
  seeds; a dormant torrent whose files were deleted reports `error` with reason `missing_files`
  when acted on, not before.
- *Tests:* `daemon.py::{lazy_restore_no_checks, dormant_seed_checks, dormant_missing_files}`;
  engine unit test `restore_dormant_record`.
- *Depends on:* P1-19, P1-07.

**P1-17 Upgrade detection and `restart`** (S)
- *Do:* §3.2.12 in `src/daemon/upgrade.rs` and `build.rs`: `FBT_BUILD_ID` (short git hash plus
  `-dirty`, or `unknown`; `FBT_BUILD_ID_OVERRIDE` wins, for tests) exported at build time and
  returned as `hello.server.build` with `server.version = CARGO_PKG_VERSION`; the daemon records
  `(dev, inode, mtime)` of `current_exe()` at start and compares every 30 s from the supervisor;
  changed and not busy → `lifecycle.request_stop(StopReason::Upgrade)` (exit 75,
  `RestartForceExitStatus=75`); changed and busy → `summary.daemon.upgrade_pending = true` until
  idle; `app.restart {scope: daemon}` → drain and exit 75 now; `fbittorrent restart` sends it and
  waits for the new `hello` (Appendix F: exit 8 after 30 s).
- *Why:* version skew between a long-lived daemon and a replaced binary or a hot-reloaded plugin.
- *Accept:* replacing the binary (copy and rename) while idle makes the daemon exit 75 and systemd
  start the new build (visible in `hello`); with a download running, `upgrade_pending` is true and
  no restart happens until idle.
- *Tests:* `daemon.py::{upgrade_idle_exit_75, upgrade_pending_while_busy, restart_now}` with two
  builds (the second differs only in a test-only build id set through `FBT_BUILD_ID_OVERRIDE`).
- *Depends on:* P1-04, P1-12.

**P1-18 Agent event stream and raw API verb** (S)
- *Do:* `fbittorrent events [--follow] [--since-seq N] [--kinds a,b] --json` connects, subscribes to
  the `events` topic with `since_seq` (A.7), filters by kind client-side, and prints one JSON object
  per event on stdout; without `--follow` it prints the replay and exits 0; with `--follow` it
  continues until EOF or SIGINT (exit 0). Exit 4 when the daemon is down, 2 on a bad kind. Each
  printed event carries `seq` and `instance` so an agent can resume with `--since-seq`.
  `fbittorrent api <op> [--arg k=v]… [--json-args '{…}'] [--ref r]` sends one operation and prints
  the result envelope unchanged (errors map to exit codes through Appendix A.3). `api --list
  [--json]` prints every operation with its args, result, `mutating`, `idempotent` and `since` from
  `api::ops::TABLE` (`src/api/ops/mod.rs`), the compiled-in table the CLI also uses to decide
  whether an op may start the daemon when it is down (`hello` is not reachable then); `robot-docs
  api` prints the table (the same content `docs/IPC.md` documents). A unit test asserts the table
  matches `docs/IPC.md`'s op list, and `api nope` suggests the nearest names from it. Both verbs
  are listed in `capabilities` as `json: true, mutating: <per op>` and in `robot-docs`.
- *Why:* agents poll `triage` today; a stream ends that, and `api` makes every daemon feature
  reachable to scripts the day it lands (§6.3), which is the Σ "mega-command" principle applied to
  the daemon split.
- *Accept:* `events --follow` prints `torrent.completed` within 2 s of a fixture completing;
  `api torrents.list` returns the A.9 objects and `list --json`'s `torrents` array equals
  `cli::shapes::torrent()` applied to them (the contract-1.x shape, P1-11); `contract_version`
  becomes `1.2.1` with `changes[]` entries for `events` and `api`; `api nope` exits 2 with
  `unknown_op` and a hint listing the nearest operation names (Levenshtein ≤ 2).
- *Tests:* `agent_contract.py::{events_follow_completion, events_since_seq_replay (kill the agent,
  add two fixtures, resume with --since-seq and see exactly those two), events_resync_when_older
  (a since_seq below the ring gets reason resync and oldest_seq), events_down_exit_4,
  api_list_matches, api_list_matches_ipc_md, api_unknown_op_hint,
  api_mutating_starts_daemon_read_does_not}`; unit `api::ops::tests::table_matches_ipc_md`.
- *Depends on:* P1-08, P1-11.

**M1a Daemon behind today's UIs (v0.2.0-pre, installed, not tagged).** *Depends on:* P1-04, P1-11,
P1-12, P1-13, P1-15, P1-16, P1-17, P1-18. Demo script in §9.7; the receipt records the one-time
compat check of §3.6.

#### Phase P2: the bar popup on API v1

**P2-01 Plugin restructure and the shared API client** (M)
- *Do:* The tree of §4.1 with the manifest of §4.1 minus the `panel` kind (`kinds: ["service",
  "bar-widget"]`, `entryPoints: {service: "Service.qml", barWidget: "BarWidget.qml"}`; the `panel`
  kind arrives with P3-08, so no half-built window is exposed), the `barWidget.defaults` of §4.1
  (`binary`, `showWhenStopped: false`, `showRates: false`, `middleClick: "dashboard"`, `tuiCommand`),
  and the polling settings `refreshIntervalSec`/`idleRefreshIntervalSec` removed. `ApiClient.js`
  (`.pragma library`) implements §4.4 rules 1 to 11 exactly: `Socket { path: Quickshell.env("XDG_RUNTIME_DIR")
  + "/fbittorrent/api.sock" }`; `write(JSON.stringify(frame) + "\n")` then `flush()`; the
  bootstrap hello when `connected` is already true; the reconnect `Timer` with backoff 0.5, 1, 2,
  4, 8 s then 10 s; `SplitParser` on `\n` with a 1 MiB line guard and `JSON.parse` in `try`;
  increasing `id`s and a pending map with 20 s (100 s for `torrents.add`) timeouts; resend of
  pending idempotent mutations (A.1) with the same `ref` after a reconnect; 45 s silence resets the
  session; `hello {proto: 1, client: {name: "oliver.torrents", version: <manifest version>, kind:
  "bar"}, interactive: false}`; `app.client {interactive}` on dashboard visibility and `app.client
  {glance}` on popup open/close; subscriptions per rule 9 (`summary` at 5,000 ms idle; `summary`
  and `torrents` at 1,000 ms while open) resubscribed after every reconnect; `online = false` with
  the backoff capped at 10 s and no automatic start; revision checks (a gap resubscribes the topic,
  a new `instance` resubscribes all). `Service.qml` is the `service` singleton holding the one
  `ApiClient`, the `Store` (P2-02 fills it) and the action functions of §4.4 (`pause`, `resume`,
  `toggleSeed`, `remove(hash, deleteFiles)`, `addFromClipboard`, `add(source, stage)`,
  `commitStaged`, `discardStaged`, `openTarget`, `setTurtle`, `recheck`, `quit`, `startDaemon`,
  `dismissClipboard`), each mutation with `ref = "qs-" + startMillis + "-" + n`; `startDaemon`
  runs `["fbittorrent", "up"]` once through `Util.execArgv`. `BarWidget.qml` reads the service
  (today's `Panel.qml` icon and popup logic moved over unchanged until P2-03 and P2-04 rewrite
  them); the service exposes `popupOpen: bool` (set by the popup's open and close; it drives the
  `torrents` subscription of rule 9 and `app.client {glance}`) and `dashboardVisible: bool` (rule
  8). Test hooks (§4.17) on the one `IpcHandler` with target `oliver.torrents`, which lives in
  `BarWidget.qml` (Quickshell allows one handler per target) and calls into the service: `state()`
  returns `{online, instance, subs: {<topic>: <last rev>}, pending: [<ref>], view: null}` as JSON
  (unguarded, like `cursor()`); `useRuntimeDir(path)` takes an `XDG_RUNTIME_DIR`-style root and
  repoints the socket to `<path>/fbittorrent/api.sock` until `useRuntimeDir("")`, refused unless
  `FBT_TEST_HOOKS=1` is in the shell's environment.
- *Why:* D13; one connection per shell.
- *Accept:* the daemon counts exactly one `bar` connection with two monitors (a second, headless
  monitor can be added with `hyprctl output create headless` if only one exists; unverified on this
  Hyprland version, the test skips with a logged reason if the command fails); after a plugin
  reload the service reconnects and resubscribes within 1 s of the shell being back; `state()`
  reports the subscription and last `rev`.
- *Tests:* `daemon.py::bar_single_connection` reads `app.info`'s `clients.bar` count (A.4, P1-06)
  while the live shell is connected to a fixture daemon through `useRuntimeDir` (opt-in,
  `FBT_TEST_QS=1`); `qs-dashboard.sh::reload_reconnects` (live shell, opt-in; the harness arrives
  with P3-08, so this case is added then): touches `~/.config/omarchy/plugins/oliver.torrents/Service.qml`
  (through the symlink; §10.4 records that Omarchy's watcher is `inotifywait -m -r`, which may not
  follow a symlinked plugin folder, so the case first asserts that the shell reloaded and logs
  "symlink reload: yes/no"; if no, `setup --plugin=copy` becomes the documented development
  default and §4.15 is corrected) and asserts `state().subs` lists `summary` again within 1 s of
  the reload, with `instance` unchanged (the daemon did not restart), logging the reconnect time.
- *Depends on:* P1-08, P1-09.

**P2-02 Store reducer and fixture tests** (M)
- *Do:* `Store.js` (`.pragma library`): `apply(frame)` reduces `snapshot.begin`, `snapshot.chunk`,
  `snapshot.end`, `delta` (`changed` with `null` meaning delete the field, `added`, `removed`) and
  `state` frames (A.8) into plain objects keyed by hash (`torrents`), per-topic state objects
  (`summary`, `network`, `settings`) and per-topic `rev`; `counts` recomputed on membership or
  state change; `filter(state | "all")` and `sort(key, reverse)` over the keys of §4.6 (Added,
  Name, Progress, ↓, Size, State, ETA) on typed values; a gap (`rev != last + 1`) raises
  `resubscribe(topic)`. `Service.qml` mirrors the store into `ListModel`s with `setProperty` for
  changed fields and insert/remove only on membership change (§4.4 "The Store"); sorting runs on
  membership change, on a sort-key change, and at most once per second for rate sorts. The fixture
  runner `plugin/tests/store/shell.qml` (run as `qs -p plugin/tests/store`, which loads
  `<dir>/shell.qml`) loads `Store.js` and
  every `contracts/ipc/v1/*.ndjson`, applies each `s2c` frame, compares with each `expect` model
  (deep equality after sorting keys), prints one `FBTQS-STEP {json}` line per frame and
  `FBTQS-PASS <file>` / `FBTQS-FAIL <file> <diff>`; `plugin/tests/qs-store.sh` wraps it in
  `plugin/tests/qs-log.py` (P0-15) and adds a timing line for sorting 500 synthetic rows.
- *Why:* correctness of the most delicate QML code, tested without a shell.
- *Accept:* every `expect` in the fixtures passes (including `subscribe-gap.ndjson`); sorting 500
  rows takes under 2 ms (otherwise move sorting server-side and log the decision in the receipt).
- *Tests:* `qs-store.sh` (`FBT_TEST_QS=1`): one `FBTQS-PASS/FAIL` per fixture group plus a timing
  line for the sort; the JSONL conversion of §9.1.
- *Depends on:* P1-05, P2-01.

**P2-03 Popup rewrite** (L)
- *Do:* `BarWidget.qml`'s `KeyboardPanel` popup per §4.5 (`Style.space(380)` wide, height fitted
  up to `Style.space(560)`, `content: PanelKeyCatcher { … }` built on first open): `PanelHero`
  (title "Torrents", the rates meta in uppercase caption, the "2 DOWNLOADING" pill, trailing
  controls add `󰐕` · window `▢` · turtle `󰢃` · quit `󰐥`), `PanelSeparator`, the `Banner`
  component for §4.11 banners (urgent tint for VPN and disk), `PanelSectionHeader` per group,
  `components/PopupRow.qml` per §4.3.1 (glyph, name with middle elision, `ProgressRule`, the meta
  line built from present fields only in the §4.3.1 order, the "Ready to watch · N min buffered"
  accent badge with [Open] after P5-05), collapsed groups "QUEUED 1 · PAUSED 1 · DONE 8 ›" expanding
  in place to at most 10 rows, the `KeyHint` caption row ("a add · / search · w window · t terminal
  · z turtle", §4.2.3; hidden until the first key press of this opening, so the popup at rest has
  the Wi-Fi panel's structure), the inline confirm strip of §4.9 ("Remove '<name>'? Files are kept."
  [Cancel] [Remove], Cancel focused, `y` confirms), the quick-add offer strip of §4.8 ("Add
  '<name>'? [Add] [Cancel]", "Already in your list: <name>", "Clipboard has no torrent" for 3 s),
  pending rendering per §4.10 (dimmed italics verb, 8 fps spinner frames or static "…" under
  `Style.reduceMotion`), the §4.11 popup states (not running with "Enter Start · w Open window",
  Starting…, Connecting…, Reconnecting… (retry in N s), protocol mismatch, version skew, parked,
  engine failed, no torrents, result unknown, upgrade pending, idle countdown as a dim line under
  the hero, stopped recently), motion through `Style.duration` only (§4.13), and keys per Appendix
  B's Popup column (`j`/`k`, Enter opens the file, `o`, `p` and Space pause, `u`, `d`/`x`/Delete →
  strip, `a`/right click → offer strip, `/` → dashboard Search, `w` → dashboard, `t` → TUI, `z`,
  `q` closes the popup; `Q` and the hero's power icon `󰐥` are the owner's off switch (A1): with
  nothing active `app.shutdown {reason: "user"}` at once, with transfers active the inline strip
  of §4.5, "Stop fbittorrent? N downloads pause until it starts again." [Cancel] [Stop], Cancel
  focused; Esc closes).
  Enter and Space are told apart with `returnRequested` (§4.12). The service sends `app.client
  {glance: true}` on open and `false` on close. The popup's `IpcHandler` keeps
  `open/close/toggle/refresh/addClipboard/tui` and `cursor()` (`Panel.qml:124-139`). No `Timer` or
  `Process` outside `Service.qml`.
- *Why:* W3; the owner's look-and-feel ask.
- *Accept:* every popup state of §4.11 and every row state of §4.3.1 is reached against the fixture
  daemon (`daemon.py --serve-fixture <state>`) and captured in dark and light (P2-08); the updated
  `widget_keys.sh` passes; the lint of P2-05 passes; no `Timer` or `Process` exists outside
  `Service.qml`; `q` closes the popup and leaves the daemon running; the power icon (and `Q`) with
  an active download shows the strip with Cancel focused, Enter cancels, and Stop drains the daemon:
  `fbittorrent status --json` reports `running: false` and the icon leaves the bar within 10 s (the
  stopped-recently dim).
- *Tests:* `regression_tests/widget_keys.sh::W-001…W-015` (cursor moves, actions, confirm strip
  Cancel-first, `z`, `w`, Esc layers; W-013 presses `a` with a magnet on the fake clipboard and
  asserts nothing is added until Enter and that the strip names the `dn=`; W-014 presses Space on
  a row and asserts a `pause` mutation, not an open; W-015 presses `q` and asserts the popup closed
  and `fbittorrent status --quiet` still exits 0; W-016 clicks the power icon with a download
  active, asserts the strip with Cancel focused, presses Enter and asserts nothing stopped, then
  presses `y` and asserts `status --json` → `running: false` and the icon gone within 10 s), each
  logging `cursor()` and `state()`;
  `qs-visual.sh popup <state>` captures for every §4.11 popup row; the lints.
- *Depends on:* P2-02, P1-12, P2-08.

**P2-04 Bar icon states** (S)
- *Do:* `BarWidget.qml`'s `BarIconButton` per the §4.5 icon table (glyph, colour, tooltip, rate
  text): `󰇚` dim for not running (when shown), idle and stopped recently; `󰇚` in `bar.foreground`
  while downloading; `󰕒` for seeding only; play-circle-outline for an unacknowledged ready torrent
  (`Torrent.playable.acked == false`, P5-05 supplies the field; until then the row is inert);
  `󰦝` in `bar.urgent` while waiting for the VPN; `󰀦` in `bar.urgent` for an error; the tortoise
  glyph appended while `summary.alt` is true. `Glyphs.js` pins every codepoint by role in one
  place: download `U+F01DA`, upload `U+F0552`, paused `U+F03E4`, done `U+F012C`, VPN `U+F099D`,
  error `U+F0026`, add `U+F0415`, power `U+F0425`, refresh `U+F0450`, terminal `U+F018D`, play
  `U+F040A`, folder `U+F0770`, trash `U+F01B4`, turtle `U+F0883` (the glyphs of §4.2.4); the
  metadata, checking, stalled, queued and ready glyphs (`dots-horizontal`, `sync`, `timer-sand`,
  `clock-outline`, `play-circle-outline`) are pinned here from the installed Nerd Font's cheat
  sheet and written into `contracts/ui/states.json` as `qml_glyph` (§4.3.4). Clicks per §4.5: left
  toggles the popup, right opens it with the offer strip showing, middle per `middleClick`
  (`dashboard` or `tui`). Visibility `visible: svc.online || showWhenStopped || root.opened ||
  svc.stoppedRecently || svc.reconnecting`, where `stoppedRecently` is true for 10 s after a `bye`
  and `reconnecting` from an EOF without `bye` until the backoff reaches its 10 s cap (§4.4 rule
  10; a crash-restart shows the §4.11 "Reconnecting" icon instead of a vanish-and-return); with
  `showWhenStopped` a left click on the dimmed icon calls `startDaemon` and opens the popup in its
  Starting… state. Colour changes animate for `Style.duration(160)` only when
  `bar.foregroundAnimationEnabled`. `plugin/tests/lint.sh` gains the structure rule (no `Timer`
  or `Process` outside `Service.qml`) and the P0-11 `"s"`-suffix grep.
- *Why:* ambient state without opening anything (W3).
- *Accept:* each icon state captured dark and light; every glyph in `Glyphs.js` paints
  (`paintedWidth > 0`) in the shell font; the lint passes.
- *Tests:* `plugin/tests/qs-glyphs.sh` renders each glyph of `Glyphs.js` and of
  `contracts/ui/states.json` in a headless `qs` and logs its painted width (including `◌`,
  §4.3.4); `qs-visual.sh popup stopped-recently` (the 10 s dim state after a `bye`) and
  `qs-visual.sh bar <state>` for each icon row; `plugin/tests/lint.sh`.
- *Depends on:* P2-02, P2-08.

**P2-05 Theme-token lint and light/dark check** (S)
- *Do:* `plugin/tests/lint.sh` (ripgrep over `plugin/oliver.torrents/**/*.{qml,js}`): fail on a
  literal colour (`#[0-9a-fA-F]{3,8}`, `Qt.rgba(`), a literal pixel size in `width:`, `height:`,
  `font.pixelSize:`, `implicitWidth:`, `spacing:` or `margins:` that is not `Style.space(`,
  `Style.spacing.` or `Style.font.`, a literal `duration:` that is not `Style.duration(`
  (§4.2.1), and the operation names `torrents.list` and `torrents.files` anywhere under `plugin/`
  (the plugin subscribes; those two results are the ones the 1 MiB line cap does not bound,
  §3.4.1), with an allow-list file `plugin/tests/lint-allow.txt` for the few justified literals
  (each with a reason). `plugin/tests/VISUAL.md`: the list of states to capture (every row of
  §4.11 for popup and dashboard, every row state of §4.3.1, every icon row of §4.5), one line each
  `<surface> <state> <fixture recipe>`. Captures in the owner's theme and in `catppuccin-latte`
  (`qs-visual.sh --light catppuccin-latte`).
- *Why:* principle 8.
- *Accept:* the lint passes in CI and fails on a planted `#ff0000`; captures stored under
  `target/test-logs/visual/<date>/` and archived with the receipts (§8.3), never under `docs/`;
  the README's two hero screenshots are the only committed images.
- *Tests:* `plugin/tests/lint.sh` with a negative fixture file (`plugin/tests/lint-negative.qml`,
  which must fail on each rule once); `qs-visual.sh --light catppuccin-latte`.
- *Depends on:* P2-03, P2-04.

**P2-06 Isolated Quickshell smoke test** (M)
- *Do:* `plugin/tests/qs-smoke.sh` and `plugin/tests/smoke/shell.qml` (`qs -p <dir>` loads
  `<dir>/shell.qml`, Quickshell 0.3.2) per §4.16 "Smoke against a disposable daemon". The script:
  `mktemp -d /tmp/fbt-XXXXXX` (short, so `api.sock` stays under the 108-byte socket limit) with
  the five XDG directories of P1-07 under it (`XDG_RUNTIME_DIR` 0700); a `config.toml` there with
  `network_interface = "lo"` (every engine socket, DHT included, binds to loopback, so the random
  hash never reaches the public DHT and the §9.1 "local only" rule holds), `peer_port` = a free
  port from `/proc/net/tcp`, `save_path` under the temp dir, `notify = false`; the binary
  `${FBT_BIN:-target/release/fbittorrent}`; `fbittorrent serve --idle-exit-secs 600 --no-notify`
  with that environment, readiness awaited with `fbittorrent status --quiet` (exit 0) for up to
  10 s; then `env XDG_RUNTIME_DIR=<tmp runtime> WAYLAND_DISPLAY=<absolute path of the real Wayland
  socket> FBT_TEST_HOOKS=1 FBT_SMOKE_HASH=<40 random hex> QS_NO_RELOAD_POPUP=1 NO_COLOR=1 timeout
  60 qs -p plugin/tests/smoke 2>&1`, piped through `plugin/tests/qs-log.py` (suite `qs-smoke`;
  the shim matches `FBTQS-` markers anywhere in a line and reads both streams, P0-15), so
  `Service.qml` connects to the disposable daemon from its first `hello` and never to the owner's
  (`Quickshell.env("XDG_RUNTIME_DIR")`, §4.4 rule 1; `useRuntimeDir` is for the live-shell
  harnesses). `shell.qml` loads `../../oliver.torrents/Service.qml` with `manifest: ({version:
  "smoke"})` (the host normally injects it, `shell.qml:944-950`), sets `svc.popupOpen = true` so
  the `torrents` topic is subscribed (§4.4 rule 9; P2-01 names the property), and runs the
  steps with a hard-timeout `Timer`: `add` (`svc.add({magnet: "magnet:?xt=urn:btih:" + hash +
  "&dn=fbt-smoke"}, false)`; the row appears with `state: metadata` at once, because P1-09 makes
  `torrents.add` create the ledger record from the magnet's hash and name before metadata
  arrives), `pause` (`svc.pause(hash)` → the row's state is `paused`), `resume` (→ `metadata`
  again), `remove_keep_files` (`svc.remove(hash, false)` → the row is gone), `reconnect` (the
  script kills the disposable daemon and starts it again; `state().instance` changes within 1 s of
  readiness and `state().subs` lists `summary` and `torrents` again), each a `FBTQS-STEP` line and
  one `FBTQS-PASS`/`FAIL`; a `trap` kills the daemon and removes the temp dir. `qs.Commons` is
  required, not optional: `Service.qml` uses `Util.execArgv` (§4.4), and Quickshell resolves
  `qs.*` from the config root, so the test root carries symlinks `plugin/tests/smoke/Commons ->
  /usr/share/omarchy/shell/Commons` and `Ui -> /usr/share/omarchy/shell/Ui` (a
  `QML_IMPORT_PATH` would look for `qs/Commons/qmldir`, which does not exist); a separate probe
  `plugin/tests/smoke/probe-ui.qml` imports `qs.Ui` and instantiates `PanelHero` first, so a
  failure there is recorded without killing the smoke. Record the outcome of the probe (`qs`
  version, date, PASS or the error) in `plugin/tests/README.md` (a table: file, what it tests,
  how to run it, last result) and, if the probe fails, the §4.16 fallback applies to the visual
  components only. `all.sh` gains the `FBT_TEST_QS=1` block entry for `qs-smoke.sh`.
- *Why:* tests the plugin-to-daemon path without touching the running shell [OR §8 item 9].
- *Accept:* `FBT_TEST_QS=1 all.sh` runs it green; when the owner's daemon is running,
  `fbittorrent api app.info --json`'s `clients.bar` is the same before and after the run (the
  script performs the check itself and skips it with a logged reason when no owner daemon runs or
  its `instance` changed between the readings); the probe's result is in `plugin/tests/README.md`.
- *Tests:* `qs-smoke.sh::{add, pause, resume, remove_keep_files, reconnect}` with `FBTQS-STEP`
  lines (observed text: the row's state after each step), plus `probe_ui`.
- *Depends on:* P2-02, P1-07.

**P2-07 Menu, keybinding, desktop entries, install script** (S)
- *Do:* §8.6: `scripts/install.sh` (created by P1-04) gains `--plugin=link` on its `setup` call
  (`fbittorrent setup --magnet --plugin=link`); `setup` flags `--magnet`, `--save`, `--plugin=link|copy`,
  `--dry-run`, `--uninstall`, `--purge` (Appendix F) and idempotence (a second run changes
  nothing); the menu entries in `~/.config/omarchy/extensions/omarchy-menu.jsonc` (D18:
  `torrents.open` → `fbittorrent dashboard`, `torrents.search` → `fbittorrent dashboard --view
  search`, `torrents.add` stays `fbittorrent add-clipboard`, `torrents.tui` keeps the terminal
  entry, `torrents.stop` gated on `fbittorrent status --quiet`); the desktop entries with
  `Icon=fbittorrent`; a keybinding suggestion with a chord checked free once by grepping
  `/usr/share/omarchy/default/hypr/bindings/*.lua` and `~/.config/hypr/` (SUPER+CTRL+T is taken
  [FO §2.8]; the chosen chord and the grep output go in the P2-07 receipt); `omarchy plugin
  enable oliver.torrents` and the bar placement only if missing; the README install section.
  Until P3-08 adds the `panel` kind, `torrents.open` and `torrents.search` run `fbittorrent
  dashboard`, which falls back to the TUI when the plugin manifest has no `panel` kind (Appendix
  F), so the v0.2.0 menu never points at a window that does not exist.
- *Why:* a working install story for v0.2.0.
- *Accept:* `setup --dry-run` with a temporary `HOME` lists the exact actions; a second `setup` on
  the owner's machine changes nothing; the menu's "Stop" entry is gated on `status --quiet`.
- *Tests:* `daemon.py::{setup_dry_run_lists_actions, setup_idempotent}` with a temp `HOME` and a
  `systemctl` shim.
- *Depends on:* P1-04, P1-11.

**P2-08 Design tokens, visual capture harness and contrast test** (M)
- *Do:* `Model.js` number formatting per §4.2.3 (sizes "1.1G"/"963M"/"12K" with one decimal below
  10, rates "3.1M"/"32K", ETA "12m"/"1h05"/"2d", percent "43%" with "99.5%" shown as "99%" and
  "0.04%" as "<1%", absent values "—", never "∞") with unit tests; `Model.dim(fg, bg)` as the one
  ladder of §4.2.1 (rung 1 `Qt.darker(fg, 1.4)`, rung 2 `mix(fg, bg, 0.75)`, rung 3 `mix(fg, bg,
  0.85)`, rung 4 `fg`; the first rung with WCAG ≥ 4.5:1 against `bg`); `Model.accentText(accent,
  bg)` (accent only when ≥ 4.5:1, else bold foreground, §4.2.1 "Accent text"); `Glyphs.js`
  skeleton; the `useRuntimeDir` hook wiring; `plugin/tests/qs-visual.sh` per §4.16 (fixture daemon
  through `daemon.py --serve-fixture <state>`, `useRuntimeDir`, `omarchy-shell shell summon`,
  `grim` capture to `target/test-logs/visual/<date>/<surface>-<state>-<theme>.png`, pixel-diff
  percentage against the previous capture, `--light <theme>` with `omarchy-theme-set` and
  restore, `--compare <plugin>` writing the pair image with our geometry from `state()` printed
  under it, `--dry-run`); `plugin/tests/contrast.py` computing, for every
  `/usr/share/omarchy/themes/*/colors.toml` and the owner's current theme, the §4.2.1 ladder (which
  rung wins and its hex) and the WCAG ratios of `foreground`, the computed dim and `accent` against
  `background`, failing when the computed dim or `foreground` is below 4.5:1, and writing
  `contracts/ui/themes/<theme>.toml` (a copy of each theme's `colors.toml`) plus
  `contracts/ui/themes/expected.json` (one object per theme: `{foreground, background, accent,
  dim, dim_rung, dim_ratio, fg_ratio, accent_ratio, accent_text, low_chroma}`, §4.2.1);
  `contracts/ui/states.json` in the shape of §4.3.4 (one element per row: `state`, `qml_glyph` as
  `U+XXXX`, `text_glyph`, `word`, `short`, `secondary`; the pending row with its `verbs` array).
- *Why:* the owner's complaint is look and feel; without captures and a contrast check nobody can
  tell whether a change made it better or worse.
- *Accept:* `qs-visual.sh popup downloading` produces a PNG of the live popup over the fixture
  daemon and restores the hook; `qs-visual.sh popup downloading --compare omarchy.network`
  writes the pair image with our geometry printed under it; `contrast.py` passes for every installed theme under the computed dim (or lists the failing
  ones with ratios, and the lint fails); `qs-format.sh::dim_ladder` asserts `Model.dim` returns
  the hex in `expected.json` (±1 per channel) for every fixture theme; `Model.js` formatting tests
  pass under `qs`.
- *Tests:* `plugin/tests/qs-format.sh` (formatting fixtures: sizes, rates, ETA, percent edge
  cases 99.5 and 0.04 %; `dim_ladder` and `accent_fallback` over `contracts/ui/themes/*.toml`, the
  same files `palette::tests` reads in P4-05; `states` over `contracts/ui/states.json`, §4.3.4);
  `contrast.py --self-test` with a known-bad theme (`contracts/ui/themes/_bad.toml`, foreground
  `#777777` on `#888888`); `qs-visual.sh --compare` (self-test: the pair image exists and names
  both panels); a `qs-visual.sh --dry-run` that checks every tool it needs (`grim`,
  `omarchy-shell`, `hyprctl`) and logs their versions.
- *Depends on:* P2-01, P1-07. (This item creates `contracts/ui/themes/` and `expected.json`,
  computed by `contrast.py`; the TUI's cross-check against the same files is the
  `dim_matches_fixture` test of the palette item, because `palette.rs` does not exist yet.)

**M1 First demo (v0.2.0).** *Depends on:* M1a, P1-14, P2-03, P2-05, P2-06, P2-07. Demo script in
§9.7.

#### Phase P3: features in the daemon, and the dashboard

**P3-01 Search in the daemon** (L)
- *Do:* `src/api/ops/search.rs` over `src/search.rs` (§3.7): sessions owned by the connection that
  started them (`search.start {query (1..256), providers?, limit? ≤ 5000}` → `{search_id}`; at most
  4 concurrent sessions, the fifth is `busy`; a session ends 30 s after its connection drops and its
  results are retained for that grace so a reloaded dashboard can resubscribe); results stream on
  the `search:<id>` topic (A.7, keyed by `key`, 500 ms default interval) as `SearchResult` objects
  (A.9, with `providers: [str]` and `added: hash?` filled from the ledger); `search.done {search_id,
  total, warnings}` (A.8); `search.stop`, `search.providers`, `search.set_provider`, `search.install
  {curated? | sources?}` (pending, then `mutation.result {installed, failed}`),
  `search.browse_sources`, `search.browse {source}` (bounded reads: the existing 16 MiB cap,
  `src/browse.rs:60`), `search.grab {search_id, key | result, …add args}` (as `torrents.add` with
  `origin: "search"`; refused with `engine_unavailable` and the VPN hint while parked, §3.2.9).
  Providers run as today (bounded capture, process-group kill, `PR_SET_PDEATHSIG`,
  `src/search.rs:583-688`) as children of the daemon, and `shutdown_all` (`src/search.rs:945-961`)
  runs on drain. Dedupe key: btih from the magnet or `.torrent`; else the normalized `(title,
  size)` pair; keep the best-seeded entry and list every provider (§3.7 "Dedup"). Mojibake repair
  per §3.7 (re-decode when the Latin-1 round trip yields valid UTF-8; the raw bytes are kept for
  the session). Fake providers for tests: `regression_tests/fixtures/providers/{alpha,beta}.py`
  (nova-style scripts printing fixed results with a duplicate pair across them and one mojibake
  name, `screens/07-search-sintel-16s.txt`'s sample). `caps` gains `search`.
- *Why:* A4; D15.
- *Accept:* fake local providers produce one deduplicated result listing both providers, with the
  name repaired; a disconnected client's session stops after 30 s and its process group is gone; a
  fifth concurrent session gets `busy`; a client that reconnects within 30 s resubscribes and gets
  the retained results.
- *Tests:* `contracts/ipc/v1/search.ndjson`; `daemon.py::{search_dedupe_and_repair,
  search_session_grace, search_fifth_busy, search_process_group_gone}` (the last reads `/proc` for
  the provider pgid); unit tests `search::tests::{dedupe_key_btih, dedupe_key_title_size,
  mojibake_roundtrip}`.
- *Depends on:* P1-08.

**P3-02 RSS in the daemon** (M)
- *Do:* `src/api/ops/rss.rs` over `src/rss.rs`: `rss.feeds`, `rss.add {url, name?}`, `rss.remove
  {feed}`, `rss.refresh {feed?}` (pending), `rss.items {feed, unread_only?}`, `rss.set_read {feed,
  item, read}`, `rss.mark_feed_read {feed}`, `rss.grab {feed, item, …add args}` (as `torrents.add`
  with `origin: "rss"`; marks read only after a successful add) with the `Feed` and `FeedItem`
  objects of A.9; topics `rss.feeds` (keyed by `id`) and `rss.items:<feed>` (keyed by `id`), on
  change (A.7). Refresh each feed once when a client becomes interactive and its
  `last_refresh_at` is older than 30 minutes (§3.7 "Scheduling"); never in the background. Reload
  `rss.json` (`src/rss.rs:69-71,93-100`) when its mtime changed before serving. Feed reads capped
  at 4 MiB (an oversized feed is refused with `upstream` and the cached items stay). `caps` gains
  `rss`.
- *Why:* A4.
- *Accept:* a local feed fixture refreshes once per interactive connect when stale; a CLI edit
  (`fbittorrent rss add` in-process while the daemon runs) appears in the topic within 2 s; an
  oversized feed is refused with a message and the cached articles stay.
- *Tests:* `contracts/ipc/v1/rss.ndjson`; `daemon.py::{rss_refresh_on_interactive,
  rss_cli_edit_visible, rss_oversized_refused, rss_grab_marks_read_only_on_success}`; the existing
  8 RSS unit tests stay green.
- *Depends on:* P1-08, P1-12.

**P3-03 Shows in the daemon** (M)
- *Do:* `src/api/ops/shows.rs` over `src/shows.rs`: `shows.list`, `shows.follow {query | imdb,
  quality?}`, `shows.unfollow {imdb}`, `shows.check {imdb?}` (pending, then `shows.checked {imdb?,
  new, warnings}`), `shows.releases {imdb | inbox: true, unread_only?, quality?}`, `shows.set_read
  {imdb, key, read}`, `shows.mark_show_read {imdb}`, `shows.undo_read` (one level, the last read
  mark), `shows.grab {imdb, key, …add args}` (`origin: "shows"`), with the `Show` and `Release`
  objects of A.9 (`Release.transfer` from the ledger by btih); topics `shows` (keyed by `imdb`) and
  `shows.releases:<imdb>` / `shows.releases:inbox` (keyed by `key`, `<imdb>/<key>` in the inbox),
  on change. The check scheduler: on interactive connect, check each show whose `last_check_at`
  is older than 30 minutes (`Show::is_stale`, `src/shows.rs:115`); never otherwise; a check never
  downloads. Reload on `(mtime, size, inode)` change of `shows.json` so an in-process CLI edit
  while the daemon runs with `--no-start` appears in the topic (§3.7). `FBT_EZTV_BASE_URL`
  overrides the hard-coded `https://eztv.re/api/get-torrents` (`src/shows.rs:547`) so tests can
  point it at the EZTV fixture server (`regression_tests/fixtures/eztv/*.json`, served by
  `fixtures.eztv_server()`). `caps` gains `shows`.
- *Why:* A4; W2.
- *Accept:* inbox semantics match the 11 Shows tests and the TUI read model (`m`, `M`, `u`); a
  check never downloads; a check with the daemon idle and no UI does not run (only on interactive
  connect or explicit `shows.check`).
- *Tests:* `contracts/ipc/v1/shows.ndjson`; `daemon.py::{shows_inbox_semantics (the step sequence
  P3-13 replays: four unread newest first, `set_read` one, `mark_show_read` the inbox, `undo_read`,
  grab marks read), shows_check_never_downloads, shows_check_only_when_asked, shows_cli_edit_visible}`;
  existing unit tests.
- *Depends on:* P1-08, P1-12.

**P3-04 IMDb service in the daemon** (S)
- *Do:* `src/api/ops/imdb.rs` over `src/imdb.rs`: `imdb.lookup {name | id, refresh?}` → `{card:
  ImdbCard?}` (`refresh: true` re-downloads the ratings index and reloads it before answering,
  for `imdb --refresh`, P3-07)
  (A.9: `id, title, year?, kind, rating?, votes?, stars?, blurb?, wiki_url?, url, poster_path?`);
  the ratings index (`title.ratings.tsv`, 28.7 MB) loaded lazily once per daemon life
  (`src/imdb.rs:117-183`); posters cached under `~/.cache/fbittorrent/posters/<id>.jpg`
  (`src/imdb.rs:101-115`) written to a temporary name and renamed (today `std::fs::write` to the
  final path, `src/imdb.rs:111`, which a concurrent in-process CLI lookup can expose half-written
  to QML); read caps (2 MiB for the suggestion JSON, 8 MiB per poster; over → `upstream` and the
  card renders without a poster). `caps` gains `imdb`. `FBT_IMDB_BASE_URL` overrides the suggestion endpoint, the poster
  CDN and the Wikipedia summary host (`src/imdb.rs:239,244`) in one go, so tests can point them at
  the fixture. Fixture: `fixtures.imdb_server()` serving
  `regression_tests/fixtures/imdb/{suggest.json, poster.jpg, poster-oversized.jpg, wiki.json}`.
- *Why:* A4; one copy of a 28.7 MB index instead of one per process.
- *Accept:* two lookups load the index once (log line `imdb index loaded`); `poster_path` exists
  and is under 8 MiB; an oversized poster fixture is refused and the card renders without a poster.
- *Tests:* unit with a fixture TSV (`imdb::tests::ratings_lookup`); `daemon.py::{imdb_index_loaded_once,
  imdb_poster_cap, imdb_poster_atomic (a reader never sees a partial file)}` with the local HTTP
  fixture standing in for the suggestion endpoint and the CDN.
- *Depends on:* P1-06.

**P3-05 Settings operations** (S)
- *Do:* `app.settings.get` → `Settings` and `app.settings.set {any subset}` → `{applied, restart}`
  (A.4) in `src/api/ops/app.rs`, validated by `Config::validate` (`src/config.rs:95`) and refused
  with `invalid_args` naming the key; live settings apply at once (`max_active_downloads`,
  `save_path`, `seed_by_default`, `idle_exit_secs`, `notify`, `handler`,
  `clipboard_watch`, `open_command`, the limit keys); restart-class settings (`peer_encryption`,
  `network_interface`, `peer_port`, listed in `Settings.restart_class`) write the config and
  restart the engine in place (`engine.changed` twice, connections stay, §3.2.8); the `settings`
  topic (A.7, `state` frames on change); every `Settings` key is the `config.toml` key (A.9).
  `caps` gains `settings`.
- *Why:* the dashboard's Settings sheet and the CLI need one path.
- *Accept:* an invalid value is refused naming the key; the queue limit applies live; an encryption
  change restarts the engine while clients stay connected (they receive `engine.changed`).
- *Tests:* `contracts/ipc/v1/settings.ndjson`; `daemon.py::{settings_invalid_named,
  settings_live_queue, settings_restart_class_keeps_clients}`.
- *Depends on:* P1-06, P1-08 (the `settings` topic and `engine.changed`).

**P3-06 Staged add** (S)
- *Do:* `torrents.add {…, stage: true}` creates a staged record (`Torrent.staged: true`, paused,
  outside the queue, excluded from `Summary.counts` and from every busy reason of §3.2.10);
  `torrents.commit {hash, files?, seed?, save_path?}` applies the choices and joins the queue;
  `torrents.discard {hash}` removes the staged record and never deletes files (A.5, §4.8 steps 1
  to 3); a staged magnet whose metadata does not arrive within 90 s stays staged with `reason:
  metadata_timeout` for the sheet's "No peers answered yet" state. `caps` gains `staging`.
- *Why:* §4.8.
- *Accept:* discarding with pre-existing files in the target folder leaves them byte-identical; a
  staged torrent does not keep the daemon alive.
- *Tests:* `daemon.py::{staged_discard_keeps_files, staged_not_busy, staged_commit_joins_queue}`.
- *Depends on:* P1-09, P1-12 (staged torrents must not count as busy).

**P3-07 CLI search, RSS and Shows through the daemon, with fallback** (L)
- *Do:* **One shape, two sources.** The CLI output of `search`, `imdb`, `rss *` and `shows *` stays
  exactly today's contract-1.x JSON (Appendix F "unchanged": the qBittorrent-shaped search results
  with `fileName`, `fileUrl`, `siteUrl`, `descrLink`, `engineName`, `nbSeeders`, `pubDate`, `imdb`;
  `rss list` as `qbt::RssFeed` with its articles, `src/qbt.rs:212-221`; `rss items` as `RssArticle`,
  `qbt.rs:162-176`; `shows list` as `Show::to_json`, `src/shows.rs:175-183`; `shows episodes` as
  `episode_json`, `shows.rs:184-197`; `imdb` as `imdb::Title`, `src/imdb.rs:17-28`). A new module
  `src/cli/shapes.rs` (add `src/cli/{mod,shapes,fallback}.rs` to §3.3; `main.rs` stays dispatch
  only) builds those shapes from the A.9 objects (`SearchResult`, `Provider`, `Feed`, `FeedItem`,
  `Show`, `Release`, `ImdbCard`), and both paths produce A.9 objects: the API path receives them
  from the daemon, the in-process path (`src/cli/fallback.rs`) calls the same functions in
  `src/search.rs`, `src/rss.rs`, `src/shows.rs` and `src/imdb.rs` that the daemon's `src/api/ops/*`
  call, which P3-01 to P3-04 make return A.9 objects. A.9 gains the additive fields the old shapes
  need (`FeedItem.description?`, `FeedItem.torrent_url?`; `Show.added_at`, `Show.counts {episodes,
  seen, grabbed}`; `Release.season?`, `Release.episode?`, `Release.season_pack`, `Release.magnet?`,
  `Release.candidates?`; `SearchResult.provider_url?`; `Provider.full_name?`;
  `ImdbCard.poster_url?`), and `Feed.id` is `rss::stable_id(url)` (`src/rss.rs:175`), so nothing
  is lossy. **Routing** (§3.2.3, §3.7): `hello` with the 200 ms timeout; if it answers with
  `state: running` and the cap the verb needs (`search`, `rss`, `shows`, `imdb`, §3.4.3), the API
  path; otherwise (no daemon, `stopping`, `parked`, a daemon that lacks the cap) the in-process
  path; in legacy mode (`engine = "qbittorrent"`, until P4-09) the WebUI path stays (`src/main.rs:644,649`).
  `rss grab` and `shows grab` move from `torrents.add` to `rss.grab`/`shows.grab` (results mapped to
  today's `{action, hash, name, feed}`). Pending operations wait as today's synchronous CLI does:
  `rss refresh` subscribes to `rss.feeds` and returns when `loading` is false for the feeds it
  refreshed (the `src/rss.rs:222` timeout); `shows check` waits for `shows.checked` and keeps
  today's `fresh`/`checked`/`failed` report and exit 5 when every show fails (`main.rs:1067-1093`);
  `search --install` waits for `mutation.result {installed, failed}`. Selectors resolve
  client-side over `rss.feeds` and `shows.list` (name substring as today, `main.rs:1152-1160`;
  `ambiguous` → exit 7, A.3). `shows seen` reads the current flag and sends `shows.set_read
  {read}` with one `ref`; `all` maps to `shows.mark_show_read` and `{count}` to today's `marked`.
  `search --limit` truncates after the shared dedupe and sort as today (`main.rs:686-701`;
  `search.start` keeps its default limit); `--wait` is complete when `search.done` has arrived; a
  `search.done` with zero results and `warnings` from every provider exits 5 as today. `imdb
  --refresh` sends `imdb.lookup {…, refresh: true}` (P3-04 adds the argument: re-download the
  ratings index and reload it in the daemon) so a running daemon never keeps a stale index. Human
  output comes from the same shaper, so it is identical too. **Orders** (Appendix F determinism):
  search results by seeders descending, then name, then first provider; feeds by `id`; items by
  `published_at` descending then `id`; shows by title; releases newest first then `key`.
  **TUI:** the "on open" refreshes move to the daemon: the real RSS refresh is the
  `rss_initial_refresh_pending` flag (`src/tui/mod.rs:193,282`, consumed at `:1921-1924` →
  `refresh_rss`) and `refresh_shows(false)` in the init batch (`:1666`; the other init commands
  at `:1664-1665` stay); remove both, because the daemon refreshes when the TUI's presence
  connection says hello (§3.7 "Scheduling"; the 30-minute rule replaces today's refresh-on-every-
  open, README:50-51, which the README now says). Until P4-03 the TUI reads `rss.json` and
  `shows.json` itself (`qbt.rs:576-578`; `tui/mod.rs:219`), so its presence connection subscribes
  to `rss.feeds` and `shows` and reloads both files when a frame arrives. The interaction test
  `rss_initial_check_and_download_dispatch_only_once` (`:3437-3450`) becomes `rss_no_initial_refresh`.
- *Why:* §3.7; no double refresh; agents keep the shapes they parse today (W5).
- *Accept:* for every verb above, the JSON and human output are byte-identical with and without a
  daemon against the same fixtures (after `warnings[]`, which only the API path can carry, is
  removed); opening the TUI triggers exactly one RSS refresh per stale feed and none for a fresh
  one; a running daemon's `imdb --refresh` reloads the index once (log line).
- *Tests:* `daemon.py::{cli_search_both_paths_identical, cli_rss_both_paths_identical,
  cli_shows_both_paths_identical, cli_imdb_both_paths_identical}` (each runs the verb against the
  fixture servers twice, daemon up and down, and diffs stdout; the fixtures are P3-01's fake
  providers, P3-02's local feed, P3-03's EZTV server, P3-04's IMDb server);
  `daemon.py::tui_open_one_refresh` (feeds made stale by editing `last_refresh_at` in `rss.json`;
  the fixture server counts hits per feed: exactly one for the stale feed, none for the fresh one);
  `agent_contract.py::{search_json_shape, rss_json_shape, shows_json_shape, imdb_json_shape}`
  (stored goldens of today's shapes under `regression_tests/golden/cli/`, created from the v0.1.2
  binary); the interaction test `rss_no_initial_refresh`.
- *Depends on:* P3-01, P3-02, P3-03, P3-04, P1-11, P1-15.

**P3-08 Dashboard window shell** (M)
- *Do:* Add the `panel` kind to the manifest (`entryPoints.panel: "Dashboard.qml"`, §4.1);
  `Dashboard.qml` per §4.6: `open(payloadJson)` creating or showing the `FloatingWindow` titled
  `fbittorrent` (`minimumSize` 900×560, `implicitWidth/Height` 1600×900), the payload routing
  (`{view, query, hash, filter}`), `close()` versus `dismiss()` (`shell.hide("oliver.torrents")`),
  the surfaces of §4.6 (`Commons.Color.background`, `Border.controlSpec("normal")` pane frames),
  the focus ladder (`forceActiveFocus`, `requestActivate`, foreign-toplevel `activate`, Hyprland
  focus dispatch, give up after 1.5 s with a log line), `app.client {interactive: true|false}` on
  visibility, the "not running" empty state with [Start] → `startDaemon` (§4.4), the view tabs
  `1`-`4` and Tab (§4.12) with empty placeholder views until P3-09 to P3-15 fill them, the
  Hyprland title-rule note in the README, the keybinding from P2-07; the `summonView(json)` test
  hook (§4.17) and `state().view = {window: {visible, width, height, view}, cursor, selection:
  [hash], filter}`. Create `plugin/tests/qs-dashboard.sh`: the harness for every dashboard case
  (summons through `omarchy-shell shell summon oliver.torrents '<json>'` or `summonView`, drives
  keys with `wtype`, reads `state()`, resizes with `hyprctl dispatch resizewindowpixel`, logs
  through `qs-log.py`; opt-in `FBT_TEST_QS=1`, live shell). Assert summoning works with the plugin
  referenced only from the bar layout (§4.1, `PluginRegistry.qml:148-163`).
- *Why:* A3.
- *Accept:* `omarchy-shell shell summon oliver.torrents '{"view":"search","query":"sintel"}'` opens
  the window on Search with that query and keyboard focus inside (typing through `wtype` reaches the
  field); 50 open and close cycles leave the daemon's open file count flat (Sprout's test
  [OR §2.1]); `app.client {interactive}` follows visibility (daemon `busy_reasons`).
- *Tests:* `qs-dashboard.sh::{summon_focus, open_close_50_fd_flat, interactive_follows_visibility}`
  (opt-in, live shell), logging fd counts from `/proc/<pid>/fd` and the focus ladder's step count.
- *Depends on:* P2-01.

**P3-09 Torrents view** (L)
- *Do:* `views/TorrentsView.qml` and `components/{TorrentTable,PaneFrame,StateText,ProgressRule,KeyHint,EmptyState}.qml`
  per §4.6: the top bar (view tabs with accent-dot badges, `FilterInput`, rates, the turtle
  indicator after P5-04, the VPN pill when binding is configured); the sidebar (`Style.space(200)`,
  the fixed filter order All, Downloading, Ready, Queued, Paused, Done, Seeding, Errors, Waiting
  for VPN with live counts from `Summary.counts`, zero counts dim and selectable, `selectedFill`
  for the active filter); the table of §4.3.2 (columns glyph 26, STATE 110, NAME ≥ 210, SIZE 76,
  PROGRESS 120, ↓ 80, ↑ 80, ETA 64, PEERS 56, RATIO 56, ADDED 80 in `Style.space`; row
  `popupRowHeight`, header `Style.space(30)`, cell padding `Style.space(10)`; glyph + word from
  `contracts/ui/states.json`; `Style.selectedAccentFill` cursor with the `Style.space(3)` accent
  bar; `selectionFill` ranges; the text progress bar; double-click is Enter; no context menu) with
  the width tiers of §4.3.3 (≥ 1600 all; 1400–1599 hide ADDED, RATIO, ↑; 1100–1399 details as a
  sheet; 900–1099 the chip row; `hideColumns` computed once per width change); sort with `s`/`S`
  and header click on typed values (§4.6 "List"); the `/` filter; the status rows (KeyHint row and
  the last message with its time); remove dialogs through `ConfirmDialog` with `selectedIndex: 0`
  and `y` mapped to `confirmed()` before `handleKey` (§4.9); pending rendering (§4.10); pane focus
  with `h`/`l` and the accent underline (§4.6 "Pane focus"); `q` closes the window, Esc steps back
  and does nothing at the top level (§4.12); the keys of Appendix B's Dashboard column whose items
  have landed by P3 (move, page, first/last, `1`-`4`/Tab, `h`/`l`, `/`, Esc, `q`/`Q`, Enter and
  double-click, `o`, `p`/Space, `u`, `d`/`x`/Delete, `D`, `s`/`S`, `f`, `i`/`w`, `y`; `a`/Ctrl+V
  arrive with P3-11, `[`/`]` with P3-10, `?`, `,` and `L` with P3-15; `z`, `R`, `P`/`K`/`J`,
  `U`/`X` and `e` with P5-04, P5-03, P6-05, P6-08 and P6-01); the sidebar buckets are
  `Summary.counts` (A.9): Downloading = `downloading + stalled + metadata + checking`, the others
  one key each, Ready present from the start and 0 until P5-05; `state().view` gains `columns:
  [{name, width, hidden}]` (names `glyph, state, name, size, progress, down, up, eta, peers,
  ratio, added`; widths in device pixels, the live `width` of the header cell), `sort: {key,
  reverse}` (keys as the `s` cycle spells them, lowercase), `toast: {text, until} | null`; the
  progress cell is eight block characters (`█▉▊▋▌▍▎▏` fractions, `Style.space(120)` minus the
  percentage) and the pending spinner is the braille set `⠋⠙⠹⠸⠼⠴⠦⠧` at 8 fps (§4.10); the turtle
  and VPN glyphs are §4.2.4's; the details slot renders `PaneFrame` with P3-10's empty state
  until that item lands, and clicking the last message does nothing until P3-15's Log view.
- *Why:* W1 and W8 in the dashboard.
- *Accept:* with 200 fixture torrents, 10 s of 1 Hz deltas plus continuous wheel scrolling
  produce no frame over 32 ms in the test instance's log: `qs-dashboard.sh` starts the test
  shell with `QSG_RENDER_TIMING=1` and parses the per-frame `render` lines Qt Quick prints to
  stderr (`time in renderer: … ms`), taking the slowest; P3-17 reuses this. The "no JavaScript
  loop per delta" lint passes; every §4.11 state that exists at P3 (the P5/P6 rows wait for their
  items); Appendix B keys;
  resizing the window through 1599, 1399 and 1099 px hides the documented columns and undocks the
  details pane at 1399; NAME is never narrower than 210 px at any width ≥ 900 (asserted by
  `qs-dashboard.sh::width_tiers` reading the header cell widths through the `state()` hook);
  Enter right after opening a remove dialog cancels (with the pointer parked outside the dialog,
  §4.9); `q` closes the window and Esc at the top level does not.
- *Tests:* `qs-dashboard.sh::{table_200_rows (parses the render-timing log and logs the slowest
  frame), width_tiers (one step per tier boundary ± 1 px, logging window width, list width and
  NAME width), confirm_cancel_first, sort_persist, q_closes_esc_does_not, double_click_is_enter}`
  (resizing through `hyprctl dispatch resizewindowpixel` after `hyprctl dispatch setfloating` on
  the window, which the harness does first; the 200 torrents are the fixture state
  `regression_tests/fixtures/states/table-200.json`, served by `daemon.py --serve-fixture
  table-200`, whose seed/leech pair is throttled so rates change every second; wheel scrolling
  is a `scroll(dy)` test hook added to §4.17 here, called 20 times per second, because `wtype`
  has no wheel; the frame time is the `time in renderer` field of each `QSG_RENDER_TIMING` line;
  the "no JavaScript loop per delta" lint is a `plugin/tests/lint.sh` rule: no `for`, `while` or
  `.forEach(` inside the frame handlers of `Service.qml` and `views/*.qml`, loops live in
  `Store.js` only); fixture `expect` models for the
  sidebar counts; `qs-visual.sh dashboard <state>` captures.
- *Depends on:* P3-08, P2-02, P2-08.

**P3-10 Details pane: Overview, Files, Log** (M)
- *Do:* `views/DetailsPane.qml`, `components/PosterCard.qml`, and the service and store work they
  need, per §4.7 for the Overview, Files and Log tabs. **Layout:** the pane is `Style.space(360)`
  wide, docked right of the list while the window is 1,400 px or wider and a sheet over the list
  below that (§4.3.3; Enter opens the sheet, Esc closes it; the sheet is `min(window − 2 ×
  panelGap, Style.space(720))` wide on the §4.6 scrim). The docked pane follows the list cursor
  after a 150 ms debounce; its edge drags between `Style.space(300)` and `Style.space(480)` and
  stops where NAME would drop below 210 px (§4.3.2); the width persists in
  `~/.local/state/fbittorrent/dashboard.json` (`{"details_width": px, "sort": {…}}`, written by
  `Service.qml` through `Quickshell.Io.FileView` with a 500 ms debounce; panels receive no
  `settings`, so the plugin owns this file; P3-09's sort persistence uses the same file). Tabs
  `[`/`]` from the list or the pane; `h`/`l` move focus (Enter on the list focuses the pane in
  docked mode); keys route by the focused pane (Space, `a`, `n`, `o`, `/` act on the Files tab only
  while the pane has focus); Peers, Trackers and Pieces tabs exist only when `caps` has `peers`,
  `trackers`, `pieces`. **Overview:** from `Torrent` (A.9; `uploaded_bytes` is added to `Torrent`
  for this tab) and one `torrents.get` on open for the `TorrentDetail` extras (`file_count`,
  `piece_count`, `piece_size`, `comment`, `created_by`, `private`, `undo_until`; refetched on
  `engine.changed`), plus `imdb.lookup {name}` once per torrent (debounced 300 ms, cached per
  `imdb_id` for the session in `Service.qml`; a `null` card shows nothing): poster from
  `poster_path` as `file://` (`Image { asynchronous: true; sourceSize }`), title, year, rating,
  blurb, `i` IMDb and `w` Wikipedia (`openUrl` → `xdg-open`); state word and the reason text of
  §4.3.4; the "Ready to watch · 4 min buffered" / "may stall" line only when `playable` is present
  (P5-05 adds the field); size, progress, `completed_bytes`, `uploaded_bytes`, ratio, ETA
  (`Model.js` formats); save path (copy) and content path (open with `Settings.open_command`,
  default `xdg-open`); info hash (copy); added and completed as `Model.when(ts)` ("12:41 today",
  "Oct 12", "2025-10-12"); the seed `ToggleSwitch` (`u`, `checked` bound to the confirmed `seed`,
  `busy` while pending) with the §4.7 meaning text ("upload capped at 32 KiB/s while downloading;
  stops when complete", "seeding allowed", or "upload capped at N KiB/s" for `upload_policy:
  custom`); queue position; an action row (`Button`s: "Recheck…" from P5-03, "Move…" from P7-02).
  **Files:** the `torrent:<h>.files` topic (own `subscribe` call; `Store.js` gains the keyed
  reducer), rows sorted by `File.sort_key` (A.9 defines it so a plain string compare is natural
  order), ☑/☐ text checkboxes (the shell has no Checkbox component), Space → `torrents.set_files
  {hash, include: [i]}` or `{skip: [i]}`, `a`/`n` → include or skip the filtered subset (an empty
  set is allowed; the toggles are disabled while the torrent is `checking`, which the engine
  refuses), per-file progress, `o` → `torrents.open_target {hash, file}`, a case-insensitive
  substring filter field (`/` while the pane has focus), "resolving metadata…" before metadata.
  **Log:** P3-15's `LogView` component embedded with `hash` pre-filtered over the one `log`
  subscription the service holds while any log consumer is visible. **Topics:** the detail topics
  are subscribed when the pane shows a torrent and unsubscribed when it hides, the torrent changes
  (after the debounce) or the view switches (§3.4.7). **Service and store:** actions `setFiles`,
  `lookupImdb`, `copyText` (`wl-copy` through `Util.execArgv`), `openUrl`, `subscribeDetail(hash,
  topics)`, `unsubscribeDetail()` in `Service.qml`; `Store.js` reducers for `torrent:<h>.files`
  and the append-only `log` topic. `state().view` gains `details: {tab, hash, width, mode:
  docked|sheet, files: [index…] (display order), poster: loaded|none}`; the `setPaneWidth(px)`
  test hook (§4.17) sets the width as a drag would.
- *Why:* W1 inspection.
- *Accept:* the natural-sort fixture (`E1`..`E10`, unpadded, out of index order) renders in
  natural order; include/skip persists across a pane reopen and a daemon restart (the ledger's
  `only_files`); the poster renders from its file against the IMDb fixture server; the detail
  topics are unsubscribed when the pane hides (`app.info.subscriptions.by_topic` has no
  `torrent:<h>.*` entry); the pane width set through `setPaneWidth` survives `omarchy-restart-shell`
  and a window reopen; `[`/`]` cycle the tabs and the sheet opens at 1,399 px.
- *Tests:* `qs-dashboard.sh::{files_natural_order, files_toggle_persists, detail_topic_released,
  pane_width_persists, details_tabs_cycle, details_sheet_below_1400, overview_poster_renders,
  log_tab_filters_hash}`; fixtures: `regression_tests/fixtures/states/files-natural.json` (a
  ten-file torrent named `E1.mkv`..`E10.mkv` built by `fixtures.multi_file_fixture(names)`, added
  to `fixtures.py` here) and the IMDb fixture server with `FBT_IMDB_BASE_URL` (P3-04's override for
  the suggestion endpoint, the poster CDN and the Wikipedia summary).
- *Depends on:* P3-09, P3-04, P1-14.

**P3-11 Add sheet, drag and drop, handler mode** (M)
- *Do:* `views/AddSheet.qml` per §4.8 "The add sheet": opened by `a` or Ctrl+V (prefilled from the
  clipboard through the service's `wl-paste --no-newline` read) and by a `DropArea` on the window
  (file URLs become `{path}`, dropped text is checked for a magnet); step 1 `torrents.add {source,
  stage: true}`; step 2 the name, the size or "resolving metadata… 0:12", the file list with
  checkboxes in natural order, the seed `ToggleSwitch` defaulting to `Settings.seed_by_default`,
  the destination (default `save_path`; P6-11 adds the picker), "Already in your list" for a
  duplicate; step 3 Enter → `torrents.commit {hash, files, seed, save_path}`, Esc →
  `torrents.discard {hash}`; step 4 after 90 s without metadata "No peers answered yet." [Keep
  paused] [Discard]. The `handler = "add" | "confirm"` setting (A5, `Settings.handler`): in
  `confirm` mode `fbittorrent add` from the desktop handler sends `torrents.add {stage: true,
  origin: "handler"}` and runs `fbittorrent dashboard --view torrents --hash <h>` so the sheet opens
  on it. The sheet is a card on the §4.6 scrim.
- *Why:* deliberate downloads.
- *Accept:* the staged flow works end to end with a local magnet fixture; discard leaves files; a
  `.torrent` dropped from a file manager opens the sheet (manual check; unverified on Hyprland until
  then); in `confirm` mode the handler opens the sheet instead of adding.
- *Tests:* `qs-dashboard.sh::{add_sheet_commit, add_sheet_discard, handler_confirm_mode}`;
  `daemon.py::handler_confirm_stages`; manual DnD check logged in the receipt.
- *Depends on:* P3-09, P3-06, P3-05 (the `handler` setting).

**P3-12 Search view** (M)
- *Do:* `views/SearchView.qml`: the `FilterInput` query field focused on open and with a `query`
  payload (`search.start`); results streamed from the `search:<id>` topic into a `TorrentTable`
  variant with the columns of §5.6 wide (Name, Size, Seeds, ★, Uploaded, Provider) and the `+ In
  your list` marker for `SearchResult.added`; `s` sorts by Seeds, Size, Uploaded, Name, ★ on typed
  values; the providers sheet (`P`: `search.providers`, `search.set_provider`, `search.install`);
  browse lists (`b`: `search.browse_sources`, `search.browse`, shown as rows; P3-17 adds the grid);
  the poster card on rest (`imdb.lookup` by `imdb_id`, debounced 300 ms); Enter stages the add
  (`search.grab {…, stage: true}`) and opens the add sheet, or selects the existing torrent when
  `added` is set; `f` follows the show (`shows.follow {imdb}`); `O` opens `page_url`; `y` copies
  the magnet (`wl-copy`); the sort, cursor and empty states of §4.11 ("No results for …").
- *Why:* W1.
- *Accept:* the fake-provider fixture shows deduplicated rows with provider lists; browse lists load;
  Enter stages one torrent; a result whose btih is in the ledger shows `+ In your list` and Enter
  selects the existing torrent; `s` sorts by Seeds, Size, Uploaded, Name, ★ on typed values; the
  query field has focus when the view opens with a `query` payload.
- *Tests:* `qs-dashboard.sh::{search_stream, search_browse, search_enter_stages,
  search_added_marker, search_sort_typed, search_focus}`.
- *Depends on:* P3-01, P3-04, P3-08, P3-06 (Enter stages).

**P3-13 Shows view** (M)
- *Do:* `views/ShowsView.qml`: a two-pane layout (shows list `Style.space(260)`, releases
  fill; one pane with a breadcrumb below 1,100 px, as the TUI's narrow layout, §5.6) with the
  TUI's columns (RELEASE key, SHOW, QUAL, SIZE, SEEDS; `Release` fields of A.9) and the inbox
  ("All unread", newest first) as the first row of the shows list. Unread marker `●` (U+25CF, in
  the shell font, the glyph the TUI uses; row added to §4.3.4) before the release key; the
  transfer symbol from the shared state table (`Release.transfer`). Keys per Appendix B: `m` and
  Space toggle read, `M` marks the show (or the inbox) read, `u` undoes the last read mark
  (`shows.undo_read`), `r`/`R` check, `n`/`+` follow by name or IMDb id, `x`/`d` unfollow, `O`
  opens the source page, Enter on a release stages the add through `shows.grab {stage: true}`
  (the dashboard stages everywhere, §4.8; the TUI downloads directly, W2) and the release is
  marked read when the staged add is committed. Unread badges: the `Color.accent` dot plus count
  on the `3 Shows` tab and the sidebar link (§4.6). Unfollow opens `ConfirmDialog` with
  `selectedIndex: 0`: "Unfollow 'Slow Horses'? Its releases leave the inbox." [Cancel]
  [Unfollow]. The `state()` hook (§4.17) reports, for this view, `{view: "shows", show, cursor,
  unread_total, unread_by_show, last_read_undoable}`. Fixture: `daemon.py --serve-fixture shows`
  serves canned EZTV JSON (two followed shows, four releases, one read) on a local port and
  starts the daemon with `FBT_EZTV_BASE_URL` pointing at it (P3-03 adds that override of
  `src/shows.rs:547`).
- *Why:* W2.
- *Accept:* against the `shows` fixture the view reproduces P3-03's `shows_inbox_semantics`
  sequence step by step: four unread newest first; `m` on one leaves three and the row loses
  `●`; `M` on the inbox leaves none and the badge disappears; `u` restores exactly the last mark;
  Enter on a release opens the add sheet and committing it sets `transfer: downloading` and marks
  it read; `x` opens the unfollow dialog with Cancel focused and Enter cancels.
- *Tests:* `qs-dashboard.sh::{shows_inbox_semantics (one `state()` read per step above),
  shows_unfollow_cancel_first, shows_enter_stages}`; `qs-visual.sh dashboard shows` at 1600 and
  900 px.
- *Depends on:* P3-03, P3-08, P3-06, P3-11.

**P3-14 RSS view** (M)
- *Do:* `views/RssView.qml`: feeds (`rss.feeds` topic; `Feed.unread`, `loading`, `error` shown
  per row) and items (`rss.items:<feed>` topic; title, `published_at`, `size` only when present,
  `●` unread marker, `+` for `grabbed`); the same two-pane/breadcrumb layout rule as P3-13;
  keys per Appendix B: `m`/`M` toggle and mark all read (`rss.set_read`, `rss.mark_feed_read`),
  `r`/`R` refresh (`rss.refresh`), `n` new feed (a one-field prompt → `rss.add`), `x`/`d` remove
  with `ConfirmDialog` (`selectedIndex: 0`, "Remove feed '<name>'? Its items leave the list."),
  Enter stages the add (`rss.grab {stage: true}`) and the item is marked read when the staged add
  is committed; the "no feeds yet" empty state with the `n` hint.
- *Why:* RSS as feeds and items.
- *Accept:* the local feed fixture behaves as the TUI does (unread count, read marks, grab marks
  read only after a successful commit).
- *Tests:* `qs-dashboard.sh::{rss_items, rss_enter_marks_read_after_add, rss_remove_cancel_first}`.
- *Depends on:* P3-02, P3-08, P3-06 (Enter stages).

**P3-15 Settings sheet, Log view, Help overlay** (M)
- *Do:* `views/SettingsSheet.qml` (`,` or the menu): every live `Settings` key of A.9 as a shell
  control (`Dropdown` for `peer_encryption`, `handler`; `TextField` for
  `max_active_downloads`, `idle_exit_secs`, `save_path`, `open_command`; `ToggleSwitch` for
  `seed_by_default`, `notify`, `autostart`, `clipboard_watch`; the VPN interface from
  `app.network`'s `interfaces`), each change sent as `app.settings.set {key}` and confirmed by the
  `settings` topic; restart-class keys show "applies after restart; restarting now" and the
  connection survives (the limits fields arrive with P5-04, the port with P6-09).
  `views/LogView.qml`: the `log` topic with `level` and `source` filter chips and a `hash` filter
  from the details pane, newest last, auto-follow. `views/HelpOverlay.qml` (`?`): every row of
  Appendix B's Dashboard column read from `plugin/oliver.torrents/keymap.json` (hand-written here
  in the Appendix B schema; P4-02 generates it) grouped by scope, on the §4.6 scrim, Esc closes.
- *Why:* settings were F6-only with three fields [RC §9].
- *Accept:* each setting round-trips through the daemon; restart-class changes show "applies after
  restart; restarting now" and the connection survives; the Log view shows an injected error with
  its source; `?` lists every dashboard key of Appendix B.
- *Tests:* `qs-dashboard.sh::{settings_roundtrip_each, settings_restart_class_banner, log_view_filter,
  help_lists_every_key}`; `contracts/ipc/v1/settings.ndjson`.
- *Depends on:* P3-05, P1-14, P3-08.

**P3-16 Dashboard smoke, captures, README** (S)
- *Do:* Extend `plugin/tests/qs-smoke.sh` to every dashboard data path (one `summonView` per view
  with a fixture payload and a `state()` read); capture every dashboard state of §4.11 dark and
  light at 900, 1280, 1500 and 1600 px (one capture per §4.3.3 tier) with `qs-visual.sh dashboard
  <state> --widths 900,1280,1500,1600`; write the README dashboard section (every §4.11 state,
  every Appendix B dashboard key, the optional Hyprland float rule of §4.6, the two hero
  screenshots).
- *Why:* the screenshots must match the real UI [OR §6 gotcha 8].
- *Accept:* smoke green; captures stored; the README's dashboard section names every §4.11 state
  and every Appendix B dashboard key (`agent_contract.py::readme_dashboard_keys` greps it against
  `keymap.json`).
- *Tests:* `qs-dashboard.sh` full run; `qs-visual.sh dashboard --widths 900,1280,1500,1600`.
- *Depends on:* P3-10, P3-11, P3-12, P3-13, P3-14, P3-15, P3-17, P3-18, P2-06.

**P3-17 Poster-rich browse grid** (M)
- *Do:* `components/BrowseGrid.qml` per §6.3 row "Poster grid": for an empty query on Search, the
  browse lists (`search.browse_sources`, `search.browse`) as a grid of 2:3 poster cells (3 to 6
  columns by width, cell width `Style.space(120)`), each with title, year, ★ rating and seeds from
  `SearchResult` plus `imdb.lookup`'s `poster_path`; at most 60 posters, `Image { asynchronous:
  true; sourceSize: cell size }`; `v` toggles grid/rows; Enter stages the add (`search.grab
  {stage: true}`); the list name shown in the query box while browsing (§5.7 row 14).
- *Why:* §6.2 row 30; the owner picks films by poster.
- *Accept:* the YTS/TPB fixture lists render as a grid within 1 s with the local poster fixture;
  the `QSG_RENDER_TIMING=1` log shows no frame over 32 ms (P3-09's budget) while 60 posters load; Enter on a cell stages the
  add.
- *Tests:* `qs-dashboard.sh::{grid_renders, grid_enter_stages, grid_frame_budget}` (the last reads
  Quickshell's frame timing through `QSG_RENDER_TIMING=1` on the test instance).
- *Depends on:* P3-12, P3-04.

**P3-18 Show quality per show and inbox filters** (S)
- *Do:* `shows.set_quality {imdb, quality?}` (A.6; `null` = default) saving `Show.quality`
  (`src/shows.rs:62-64`) and used by the next check as the preferred quality; a `Dropdown` on the
  show row (2160p, 1080p, 720p, 480p, default) in `ShowsView.qml`; inbox filter chips (unread
  only; quality) driving `shows.releases {inbox: true, unread_only, quality}`; the TUI's `=` prompt
  lands in P4-07; CLI `shows quality <show> <q>` (Appendix F; through the daemon or in-process).
- *Why:* §6.2 row 29.
- *Accept:* setting 2160p on a show persists and the next fixture check prefers 2160p releases; the
  inbox filter hides read releases.
- *Tests:* unit `shows::tests::quality_persists`; `daemon.py::shows_quality_prefers`;
  `qs-dashboard.sh::shows_quality_dropdown`; `agent_contract.py::shows_quality_verb`.
- *Depends on:* P3-13, P3-07 (the CLI verb goes through the daemon with the in-process fallback).

**M2 Every step of W1, W2 and W8 works from the dashboard (v0.3.0).** *Depends on:* P3-07, P3-16.

#### Phase P4: the TUI as a pure client

**P4-01 Isomorphic split of `src/tui/mod.rs`** (L)
- *Do:* The §5.3 tree, moves only: `src/tui/{mod,app,layout,mouse,footer,help,text,theme,poster}.rs`,
  `src/tui/screens/{torrents,search,shows,rss}.rs`, `src/tui/modals/{files,providers,settings,prompt}.rs`
  (the `peers`, `log` and `details` modals are created by P5-02, P4-08 and P4-04), `src/tui/tests/*.rs`
  (the 21 interaction tests moved unchanged); `src/tui/actions.rs`, `src/tui/store.rs`,
  `src/tui/format.rs` and `src/tui/palette.rs` are created empty here and filled by P4-02, P4-03,
  P4-07 and P4-05. `mod.rs` keeps `run()` and the Model glue under 400 lines. Extend
  `scripts/lint-sizes.sh` (P1-02) to `src/tui/`.
- *Why:* §5.3.
- *Accept:* the same tests with the same names pass; golden renders at three sizes and the tmux
  captures are identical before and after; no file in `src/tui/` exceeds 900 lines.
- *Tests:* goldens (P0-16); `native.py` unchanged; `scripts/lint-sizes.sh src/engine src/tui`.
- *Depends on:* P1-15, P0-16.

**P4-02 One action table for keys, footer and help** (M)
- *Do:* §5.4 in `src/tui/actions.rs` (created empty by P4-01). Types: `KeyChord { code:
  ftui_core::event::KeyCode, mods: ftui_core::event::Modifiers }`; `Scope::{Global,
  Screen(Screen), Pane(Screen, Pane), Modal(ModalKind), Prompt}`; every `Action` also carries
  `surfaces: &'static [Surface]` (`Tui`, `Dashboard`, `Popup`) and an optional `surface_note`
  (the popup's "Enter opens the file"), so dashboard- and popup-only rows (popup `w`/`t`) live
  in the same table with `run: None`. Conditional keys are `enabled` closures: the undo toast is
  `Scope::Screen(..)` with `enabled: |a| a.toast.is_some()`; the legacy `s`/`f` toggles are
  `enabled: |a| !a.cfg.native()` until P4-09 deletes them; `y` in a prompt is `Scope::Prompt`.
  `fbittorrent robot-docs keys [--json] [--surface tui|dashboard|popup|all]` (default `tui`;
  `RobotDocs` gains the flags next to its positional `section`, `src/main.rs:56-60,322-328`)
  prints the Appendix B table in Markdown, or with `--json` the schema of Appendix B's "keymap
  schema" paragraph. `scripts/gen-keymap.sh` runs `robot-docs keys --json --surface all` into
  `plugin/oliver.torrents/keymap.json` (P3-15's hand-written first version follows the same
  schema) and rewrites the README block between `<!-- keys:begin -->` and `<!-- keys:end -->`
  (added by this item under "Keys"). Footer row 1: enabled actions of the focused scope in
  `priority` order (10 Enter/primary, 20 open and pause, 30 seed and remove, 40 add and recheck,
  90 the rest), each as `(key) Label` separated by two spaces, `…` appended when any did not fit,
  `(?) Help` always reserved at the right; row 2 is the selected item's full name and context
  (§5.5.4). Help lists every action grouped by scope in Appendix B's order, paged with PgUp/PgDn
  as today's help pages. Contract: `robot-docs keys` is listed in `capabilities.commands[]`
  (additive, 1.2.x).
- *Why:* ends the three-copy drift; one keymap for all surfaces.
- *Accept:* the invariant tests pass; the footer shows `…` when actions overflow; the README keys
  table and `keymap.json` match the generated output (test).
- *Tests:* `actions::tests::{table_covers_appendix_b (every key in `keymap.json` for the TUI
  surface is in the table and vice versa), no_handler_outside_table (`rg -c 'KeyCode::Char\('
  src/tui/screens src/tui/modals` is 0: dispatch goes through `actions::dispatch` only),
  no_duplicate_key_per_scope (a toast scope is its screen's scope, so `u` and `U` cannot both
  mean undo and seed there), readme_table_matches, keymap_json_matches,
  global_keys_never_rebound (`q`, `Q`, `?`, `1`-`4` mean the same on every screen; modals are
  not screens, so Files' `1` is allowed)}`; golden `footer-overflow-80x24.txt` with a fixture
  state that enables nine actions.
- *Depends on:* P4-01.

**P4-03 The TUI on API v1** (L)
- *Do:* §5.2 step 2: replace `qbt::Client` in `src/tui/` by `api::Client` (`src/api/client.rs`,
  P1-11) with `client.name = "tui-<pid>"`, `kind: "tui"`, `interactive: true`; `src/tui/store.rs`
  mirrors the `summary`, `torrents`, `network`, `settings`, `rss.feeds`, `shows` topics and the
  detail topics of the visible torrent with the same reducer semantics as `Store.js` (P2-02),
  tested against the same `contracts/ipc/v1/*.ndjson` `expect` models; an `ApiSub` ftui
  `Subscription` turns frames into `Msg::Api(Event)` (the `SearchSub` pattern,
  `src/tui/mod.rs:309-350`); the 2 s poll is gone; search (`search.start` and the `search:<id>`
  topic), RSS, Shows, IMDb and browse go through the daemon; `Q` sends `app.shutdown`; the
  presence connection becomes the only connection; a lost connection shows the §4.11 banners
  ("Reconnecting… (retry in N s)", "fbittorrent stopped" with Enter → `ensure_daemon()`).
- *Why:* one client contract; no polling; features from the daemon.
- *Accept:* `src/tui` no longer uses `qbt::Client` (`rg -n "qbt::Client" src/tui` is empty); no 2 s
  poll; the presence connection becomes the only connection (`app.info.clients.tui == 1` with one
  TUI); the interaction tests pass after adaptation; search, RSS and Shows behave as before;
  `store::tests` pass every `expect` model the QML runner passes.
- *Tests:* interaction tests; `store::tests::fixtures`; the native tmux smoke (P4-10 extends it);
  `daemon.py::tui_single_connection`.
- *Depends on:* P4-01, P3-07.

**P4-04 Responsive layouts at 80, 120 and 160 columns** (L)
- *Do:* `src/tui/layout.rs` with the breakpoints of §5.6 (narrow < 100, medium 100–139, wide
  ≥ 140) and the per-screen column sets of the §5.6 table (Torrents narrow: STATE 9, NAME, %, ↓
  when live, ETA; medium adds SIZE and PEERS and the selected-torrent strip; wide adds the side
  pane); the state column of §5.5.3 (`text_glyph` + `short` from `contracts/ui/states.json`,
  exactly 9 cells); rectangles recorded during render in `layout.rs` drive `src/tui/mouse.rs` hit
  testing; `ftui_widgets::table` for rendering with `sort_column: None` (sorting on typed values in
  `store.rs`, §5.8), `virtualized` for every list (no threshold), `scrollbar` on the right border of
  every list; the details modal `src/tui/modals/details.rs` (tabs Overview · Files at this item,
  Log from P4-08, Peers from P5-02, Trackers from P6-01, Pieces from P6-02; `[`/`]` stop at the
  ends; full screen at narrow, a centred 100×30 at medium that falls back to full screen under 30
  rows, on Enter at wide too; the 80×24 mockup of §5.6 is its golden); the banner slot (one line
  above the footer, the §4.11 popup wording, bold with a leading `!` on low-chroma themes,
  priority disk > VPN > stopped/reconnecting > upgrade pending). Column widths at narrow: `▌` in
  column 0, STATE 9, `%` 5, `↓` 6, ETA 6, NAME fills; medium adds SIZE 9 and PEERS 5, and the
  one-line strip above the footer (state word · reason phrase of §4.3.4 · queue position · cap
  text · path, middle-elided) is distinct from footer row 2, which at every width carries the
  selected item's full name and context (P4-02). A cell is blank when the value does not apply
  (`↓` and ETA when `dl_bps` is absent, §5.7 row 8) and `—` when it applies but is unknown (SIZE
  before metadata). `contracts/ui/states.json` is embedded with `include_str!` (a contract, not
  configuration); the rectangles are `layout::Rects { rows: Vec<(hash, Rect)>, headers:
  Vec<(Column, Rect)>, tabs, footer }` recorded per frame and read by `mouse.rs` (a header click
  sorts, as in §4.6); the width check uses `text.rs::dw` (`unicode-width`, the crate ftui uses)
  and is the one `state_glyph_width_one` test, which §5.5.3 describes. Goldens for Search, Shows and
  RSS at the three sizes and for Torrents at 160 have no mockup in §5.6: they are generated from
  the P0-16 fixture here and reviewed in the receipt; Appendix C's Search and Shows mockups at 80
  are their targets.
- *Why:* [RC §9] defects 1 and 4; the owner's window is about 90 columns wide.
- *Accept:* goldens for each screen at three sizes show state words; a click test hits each visible
  row at each size; the details modal opens with Enter at 80 columns and `[`/`]` switch tabs.
- *Tests:* goldens (the §5.6 mockups are the goldens for Torrents at 80×24 and 120×30 and for the
  details modal at 80×24); `tui::tests::click_hits_rows_at_each_size`;
  `tui::tests::state_glyph_width_one`; tmux smoke steps at 80×24.
- *Depends on:* P4-02, P4-03.

**P4-05 Contrast and low-chroma themes** (S)
- *Do:* `src/tui/palette.rs`: `palette::dim(fg, bg)` as the §4.2.1 ladder (rung 1 Qt's HSV darken
  by 1.4, value × 100 ÷ 140, hue and saturation unchanged; rung 2 `mix(fg, bg, 0.75)` per 8-bit
  sRGB channel, `round(0.75·fg + 0.25·bg)`; rung 3 the same at 0.85; rung 4 `fg`; the first rung
  with WCAG ≥ 4.5:1), `palette::accent_text(accent, fg, bg)` (bold `fg` when `accent` is below
  4.5:1 against `bg`, or when `accent` and `fg` differ by less than 32 in every channel **and** are
  within 1.3:1 of each other; §5.5.1), `palette::low_chroma(theme)` (red and green differ by less
  than 32 in every channel; §5.5.2) and the roles of §5.5.1 from `Theme::load`
  (`src/tui/theme.rs:58`); fixes D16 (`theme.rs:66-67` loads `muted` then overwrites it with
  `dark_foreground`; neither is a text colour now).
- *Why:* the owner's active theme removes hue [RC §9].
- *Accept:* the computed dim reaches 4.5:1 and `foreground` reaches 4.5:1 on every installed
  theme and on the owner's (the ratio is logged per theme; `rose-pine` 6.7:1 passes); the
  low-chroma fallback triggers on the owner's theme and not on `tokyo-night`; the accent fallback
  (§5.5.1) triggers on `catppuccin-latte`, `miasma`, `rose-pine` (below 4.5:1 on their
  background) and `kanagawa` (accent equals foreground), and not on the owner's theme (6.3:1,
  channel difference 114) or `tokyo-night`; the dim hex per fixture theme equals
  `contracts/ui/themes/expected.json` within ±1 per channel.
- *Tests:* `palette::tests::{contrast_every_installed_theme, low_chroma_detection,
  accent_fallback_table, dim_matches_fixture}` reading `contracts/ui/themes/*.toml` and, when
  present, `/usr/share/omarchy/themes/*/colors.toml` (the live sweep is skipped with a logged
  reason off Omarchy; the fixture sweep always runs).
- *Depends on:* P4-01, P2-08 (the theme fixture and `expected.json`).

**P4-06 Motion: marquee fix** (S)
- *Do:* §5.7 row 10 in `src/tui/screens/torrents.rs` and `text.rs`: no marquee on the selected
  row (its full name goes to footer row 2, P4-02); a hovered name scrolls once at 8 cells/s to its
  end, then resets to the start and stops; `reduce_motion = true` in `config.toml` (a new key,
  also in `Settings`) disables scrolling entirely; the timer is the existing 300 ms `Msg::Hover`
  tick (`src/tui/mod.rs:1673`).
- *Why:* [RC §9] defect 10.
- *Accept:* a selected long name never wraps; hover scrolls once and stops.
- *Tests:* interaction tests with a fake clock (`marquee_once_then_stops`, `reduce_motion_no_scroll`).
- *Depends on:* P4-01.

**P4-07 Remaining UX defects** (M)
- *Do:* §5.7 rows 4 (RSS at 80: episode key kept at the left, middle elision, no size column when
  sizes are unknown), 8 (rate column blank unless downloading; Size its own column), 9 (unknown is
  absent everywhere: no "0 seeds", "0s", "0 B"), 12 (the Appendix B keymap through the action
  table: `s` sort, `U` undo, `=` show quality, `P` providers, `y` copy magnet, `L` log, `,`
  settings alias, PgUp/PgDn and Home/End on every list), 14 (columns kept at 160; the browse list
  name in the query box), 17 (the tab bar shows the daemon state from `Summary.daemon`: "idle ·
  stops in 4 min", "waiting for VPN"; the theme name is dropped; the state text takes the place of
  the rate text, which it replaces only while both rates are 0 or the engine is parked, because at
  80 columns the tab row has 14 spare cells, not the 21 the text needs, and a transferring daemon
  needs no state word) and the lower-severity notes of
  §5.7 (browse lists before 1337x, `src/browse.rs:34-36`; natural sort in the files modal; F6
  gains save path, seed default, idle exit; the Shows IMDb card takes the height it needs;
  "(F)ollow show" only for TV titles). `src/tui/format.rs`: sizes, rates, ETA and percent as
  §4.2.3 defines them (the Rust twin of `Model.js`), shared with the CLI's human output
  (`fbittorrent list`). The Shows quality prompt (`=`, P3-18's TUI side → `shows.set_quality`).
  README claims of D14 restored where the code now matches (§8.5).
- *Why:* the rest of the 5.3/10 score.
- *Accept:* each row has a test or a golden; README claims of D14 restored where the code now matches;
  `fbittorrent list` and the TUI print identical size/rate strings for a fixture.
- *Tests:* goldens; interaction tests; `format::tests::{sizes, rates, eta, percent_edges}` shared
  with the CLI and asserting the same strings as `qs-format.sh`'s fixtures (`contracts/ui/format.json`,
  created here: one object per case `{input, kind, expect}`, read by both);
  `agent_contract.py::list_human_matches_tui_format`.
- *Depends on:* P4-04, P3-01, P3-18.

**P4-08 Log modal, toasts and message history** (S)
- *Do:* `src/tui/modals/log.rs`: `L` opens the daemon log (the `log` topic through `store.rs`,
  `LogEntry` of A.9) in ftui's `log_viewer` filtered to `level ≤ warn` plus the TUI's own action
  outcomes, with `l` cycling the level and `h` the source filter; transient feedback through ftui
  `toast` and `notification_queue` (two toasts stack above the footer, 10 s each, never covering
  footer row 1), with every toast also appended to the message history the modal shows.
- *Why:* [RC §9] defect 13.
- *Accept:* an action failure is visible in the modal after its toast expired; two overlapping
  toasts stack without hiding the footer.
- *Tests:* interaction tests `log_modal_keeps_failure`, `toasts_stack`; golden of the modal at
  120×30.
- *Depends on:* P4-03, P1-14.

**P4-09 Remove legacy qBittorrent mode and the compat paths** (M)
- *Do:* Delete the legacy WebUI transport and its tests (`src/qbt.rs`, `src/qbt_tests.rs`), the
  legacy TUI keys (`s`, `f` toggles, priorities 2 and 3, `src/tui/mod.rs:1331-1336` before the
  split), the legacy unit text (`src/setup.rs:7-20`) and the legacy backend functions
  (`src/backend.rs`); remove the `engine.sock` adapter (`src/api/legacy.rs`, P1-10) and the probe
  in `ensure_daemon()`; remove `src/engine/legacy_import.rs` after checking on the owner's machine
  that `~/.local/share/fbittorrent/legacy-migration.json` records a completed migration (the
  check and its output go in the receipt). Keep `seq`, `first-last` and `reconcile` as documented
  stubs (they already refuse in native mode, `src/main.rs:314-318`), so the CLI contract stays 1.x.
  `engine = "qbittorrent"` in `config.toml` becomes an error at startup naming this release.
- *Why:* the owner left qBittorrent; every method branches on it today (§1.7).
- *Accept:* `rg -n "qbittorrent-nox|engine\.sock" src plugin` finds nothing; the agent contract
  checks pass; the owner's data opens unchanged.
- *Tests:* full suite; `agent_contract.py::stubs_still_refuse_with_hint`.
- *Depends on:* P4-03, P2-01, M1.

**P4-10 Native tmux smoke at three sizes** (M)
- *Do:* Replace the legacy `regression_tests/tui_smoke.sh` with `regression_tests/tui_smoke.py` on
  `fbtlog.py`: a disposable daemon (the `daemon.py` helpers) with the fixture torrents of P0-16's
  state, a tmux session resized to 80×24, 120×30 and 160×45 in turn (`tmux resize-window`), and
  per size the steps list, state column, details modal (`[`/`]`), files modal, prompts (`d` then
  Esc), screens `1`-`4`, help, `q` (daemon keeps running), `Q` with the confirm; each step logs the
  captured pane (`tmux capture-pane -p`) as its `observed` text.
- *Why:* the strongest end-to-end UI test, kept for the new architecture.
- *Accept:* green locally and in CI; a failing step's JSONL contains the pane text.
- *Tests:* its own steps against the disposable daemon; a failing step's JSONL shape is covered by `fbtlog_test.py`'s forced-failure case.
- *Depends on:* P4-04, P1-07.

**P4-11 Poster card rules** (S)
- *Do:* §5.5.5 in `src/tui/poster.rs` and the screens: the card is 24×18 cells (a 2:3 image at
  24×36 half-block pixels, `src/tui/poster.rs:12-14`); at narrow widths `v` toggles a centred
  overlay with the card and Esc hides it; on Shows at wide widths the card takes the pane height;
  a kitty placement is removed before any overlay (help, prompt, modal) is drawn and restored
  after (`src/tui/mod.rs:2229`'s overlay check); lookups are debounced 300 ms and cached per
  `imdb_id` for the session.
- *Why:* posters are a feature the owner uses on every screen; today the card vanishes below 140
  columns and is squeezed to 9 rows on Shows.
- *Accept:* at 80 columns `v` shows the card and Esc hides it; at 160 the Shows card uses the pane
  height; opening Help over a kitty poster removes the placement (no ghost image in the capture).
- *Tests:* goldens (half-block path); `tui_smoke.py::poster_overlay_cleanup` with kitty detection
  logged.
- *Depends on:* P4-04.

**M3 TUI client (v0.4.0).** *Depends on:* P4-05, P4-06, P4-07, P4-08, P4-09, P4-10, P4-11.

#### Phase P5: T1 features

All tests log as in §9.1. Each item names its §6.3 row, which carries the daemon, API, UI and
test columns; the parameters an implementer needs are repeated inline.

**P5-01 Seed count and peer list in the engine and API** (M)
- *Do:* §6.3 row "Seed count and peer list": in the fork, `seed: bool` on `PeerStats`
  (`vendor/rqbit/librqbit/src/torrent_state/live/peer/stats/snapshot.rs:27-32`) set in the
  `From<&Peer>` impl (`snapshot.rs:57`) from the live peer's `bitfield: BF`
  (`live/peer/mod.rs:259`; the all-pieces test `has_full_torrent` at `:297-299`); in the daemon
  (`src/api/ops/torrents.rs`), `torrents.peers {hash}` → `{peers: [Peer]}` with the `Peer` fields of
  A.9 (`addr`, `client?`, `conn`, `incoming`, `state`, `downloaded`, `uploaded`, `dl_bps?`,
  `ul_bps?`, `seed?`), per-peer rates derived from successive `counters` samples one second apart
  (`per_peer_stats_snapshot`, `torrent_state/live/mod.rs:782`); the `torrent:<h>.peers` topic (A.7,
  keyed by `addr`, 1,000 ms); `Torrent.seeds` = connected peers with `seed: true` (absent until
  the engine is live for that torrent, §3.4.9); `caps` gains `peers`; `show <sel> --json` gains
  `seeds` and `peers[]` (Appendix F).
- *Why:* D6 (seeds are always 0 today) and diagnosing slow downloads (§6.2 row 8).
- *Accept:* a local swarm with one seed and one leecher reports `seeds: 1` and two peers with the
  right seed flags; `peers.ndjson` round-trips; per-peer rates move while data flows.
- *Tests:* fork unit test `peer_stats_seed_flag`; `contracts/ipc/v1/peers.ndjson`;
  `daemon.py::{peers_local_swarm, peer_rates_move}` (the seed/leech pair of P1-07).
- *Depends on:* P1-08.

**P5-02 Peers views** (M)
- *Do:* §6.3 row "Seed count and peer list", UI columns: the dashboard's Peers tab in
  `DetailsPane.qml` (§4.7: address, client, connection kind, downloaded, uploaded, rates, seed
  flag; subscribed to `torrent:<h>.peers` only while the tab shows) and the "seeds/peers" rendering
  of the PEERS column ("1/41" when `seeds` is present); the TUI's `src/tui/modals/peers.rs` tab in
  the details modal with the same columns.
- *Why:* T1 lands in both UIs (A2).
- *Accept:* both UIs show the local swarm's peers with moving rates; the topic is released when the
  tab closes.
- *Tests:* TUI goldens at 120 and 160 columns; `qs-dashboard.sh::{peers_tab, peers_topic_released}`.
- *Depends on:* P5-01, P3-10, P4-03.

**P5-03 Recheck** (M)
- *Do:* §6.3 row "Recheck". **Daemon** (`src/api/ops/torrents.rs`, `src/engine/ops.rs`):
  `torrents.recheck {hashes (1..256)}` → `{state: "pending"}` at once, then one `mutation.result
  {ref, op, outcome, result: {rechecked: [hash], failed: [{hash, message}], have_bytes: {hash:
  bytes}}}` for the call (A.8); the work runs in a spawned task outside the ledger lock and off the
  dispatch pool (§3.4.10), one hash at a time, through the existing rebuild path
  (`AppEngine::rebuild`, `src/engine.rs:262-277`, called today only from the start-error branch at
  `:756-760`; its "changing upload cap" error wording is generalised) and
  `ManagedTorrent::wait_until_initialized` (`vendor/rqbit/librqbit/src/torrent_state/mod.rs:602`)
  to know when the check ends. While a hash is being rechecked the daemon holds it in an in-memory
  `rechecking` set and the §3.5 mapping shows `checking` whatever `Initializing { paused }` says; the
  re-add keeps the record's intent (`paused: true` stays paused and never "recovers"; `run`
  resumes after the check) and a recheck never takes a download slot (the queue reconciles after
  the check). If the check finds missing data, `completion_on` is reset to 0 and `completed_at`
  cleared, so P1-16 can never restore a corrupt torrent as dormant `done` and P1-13's completion
  fires again when the repair completes. A dormant torrent (P1-16) is restored with its check,
  which is the recheck (no second check). Refusals: a staged torrent or one without metadata is
  `invalid_state`; a hash already in `rechecking` answers `pending` with the same outcome; while
  parked, `engine_unavailable`. A pending recheck is a `mutation` busy reason (§3.2.10). Resume
  files are P6-06's business (it depends on this item). `caps` gains `recheck`; the contract
  bookkeeping of the §9.3 preamble applies (`api::ops::TABLE` row: mutating, not idempotent per
  A.1; `docs/IPC.md` changelog; `contracts/ipc/v1/recheck.ndjson`). **CLI:** `fbittorrent recheck
  <sel>… [--wait] [--timeout N]` (Appendix F): exits 0 at `pending` with `{ok, action: "recheck",
  count, hashes, pending: true}` (the `act()` shape, `src/main.rs:718-728`), or with `--wait` waits
  for `mutation.result` and reports `{…, rechecked, failed}`; `all` beyond 256 hashes is sent in
  chunks of 256; exit codes 0, 1 (`invalid_state`), 2, 4, 6, 7, 8 (`--wait --timeout`). **TUI:**
  `R` on a torrent row (`Scope::Screen(Torrents)`, priority 40) opens the prompt "Recheck
  '<short(name, 50)>'? It re-reads all data. y/N" through `open_prompt` (`src/tui/mod.rs:1255-1280`;
  only `y` confirms, P0-13) and sends `torrents.recheck` over API v1; the row shows `⟳ Check`
  until the delta. **Dashboard:** `R` in the list and in the details pane, and the "Recheck…"
  `Button` in the Overview action row (§4.7), open `ConfirmDialog` with `selectedIndex: 0`,
  message "Recheck '<name>'? It re-reads all data.", `confirmText: "Recheck"`; the row shows
  "checking…" (§4.10) until the `state: checking` delta; the action is hidden when `caps` lacks
  `recheck`. **Fixture:** `fixtures.corrupt_byte(path, offset)` on a completed local payload of
  the 256 KiB magnet fixture (16 KiB pieces, so 16 pieces).
- *Why:* §6.2 row 1.
- *Accept:* with the local seed held offline, rechecking the corrupted-byte fixture ends with
  `have_bytes` below the size and the row at `checking` then `downloading`/`stalled` (never
  `done`); bringing the seed back restores it to `done` and `torrent.completed` fires again; a
  recheck of a dormant torrent (P1-16) performs exactly one check; a paused torrent stays paused
  after its check; `recheck` on a staged torrent exits 1.
- *Tests:* `daemon.py::{recheck_corrupt_byte (the seed daemon paused until `mutation.result`
  arrives, then unpaused; logs `have_bytes` and the state sequence), recheck_dormant,
  recheck_paused_stays_paused, recheck_resets_completion (restart the daemon mid-repair: the
  torrent comes back `checking`, not `done`), recheck_staged_invalid_state}`;
  `src/tui/tests/prompts.rs::recheck_prompt_defaults_no`;
  `qs-dashboard.sh::{recheck_cancel_first, recheck_overview_button}`;
  `agent_contract.py::recheck_exit_codes` (1 staged, 4 `--no-start` with the daemon down, 6 unknown
  selector, 7 ambiguous selector, 8 `--wait --timeout 1`).
- *Depends on:* P1-09, P3-10, P4-02, P4-03 (the TUI sends `torrents.recheck` over API v1), P1-16 (`recheck_dormant`).

**P5-04 Global and alternative speed limits (turtle)** (M)
- *Do:* §6.3 row "Global and alternative limits": `transfer.set_limits {download_bps?, upload_bps?,
  alt_download_bps?, alt_upload_bps?}` → `{limits}` and `transfer.set_alt {on}` → `{alt}` (A.4)
  applied live through `Session.ratelimits.set_download_bps/set_upload_bps`
  (`vendor/rqbit/librqbit/src/session.rs:140`, `limits.rs:85-91`); the config and `Settings` keys
  `download_limit_bps`, `upload_limit_bps`, `alt_download_limit_bps`, `alt_upload_limit_bps`,
  `alt_limits_on` (A.9; the turtle state survives a restart); `Summary.alt` and `Summary.limits`;
  the global upload cap never lifts the per-torrent 32 KiB/s default. `z` toggles `alt` in every
  surface (Appendix B); the popup hero's turtle button, the dashboard top-bar indicator and the
  Settings sheet's four limit fields (`TextField`, bytes per second with `K`/`M` suffixes, empty =
  unlimited); the TUI's F6 fields and tab-bar turtle glyph; CLI `fbittorrent limits [--down N]
  [--up N] [--alt on|off] [--alt-down N] [--alt-up N]` (Appendix F; `app.settings.set` when the
  daemon is down); `caps` gains `limits`.
- *Why:* §6.2 rows 5 and 6; turtle mode is one of the OmaTorrent-style capabilities the owner named.
- *Accept:* a local transfer under a 64 KiB/s alternative limit stays within ±25 % over 20 s after a
  5 s warm-up; the bar, popup, dashboard and TUI show the turtle state within one push interval;
  the per-torrent 32 KiB/s default is unaffected by a global upload limit of 0 (unlimited); the
  turtle state survives a daemon restart.
- *Tests:* unit tests on `Limits`; `daemon.py::{alt_limit_holds, global_unlimited_keeps_cap,
  alt_survives_restart}`; `qs-smoke.sh::turtle_toggle`; TUI interaction test;
  `agent_contract.py::limits_verb`.
- *Depends on:* P3-05, P2-03, P3-09, P4-02, P4-03 (F6 and `z` in the TUI set limits over API v1).

**P5-05 Ready to watch** (M)
- *Do:* §6.3 row "Ready to watch": `src/daemon/watch.rs` (prefix computation once per second per live
  incomplete torrent from P5-06's bitfield snapshot: the contiguous byte prefix of the largest
  video file, by extension `mkv mp4 webm avi mov m4v ts`, from the file's own first piece),
  `playable` in `Torrent` (A.9 `Playable`: `file`, `ready` when prefix ≥ max(32 MiB, 3 % of the
  file) and the file's last piece is present, DL-26; `stall_free` = `prefix_bytes / bitrate ≥
  (file_size − prefix_bytes) / dl_bps`; `prefix_bytes`; `buffered_s = prefix_bytes / bitrate`;
  `finishes_in_s = (file_size − prefix_bytes) / dl_bps`; `acked`), the
  `torrent.playable` event (A.8; once per torrent per daemon life), the notification "Ready to
  watch" with `default` and `open` (§3.8), the TUI state cell `▶ Ready` and details line, the popup
  badge and [Open], the dashboard sidebar "Ready" filter and row badge, the bar icon state (P2-04's
  row, driven by `acked`), `wait --until playable`, `watch` in `caps`.
  `buffered_s` only when a container duration is cheaply available
  (an `mkv`/`mp4` header read of at most 1 MiB; otherwise omitted). `stall_free` and
  `finishes_in_s` from `dl_bps`, recomputed per second (§3.5); the popup, table and TUI row text
  use the two wordings of §3.5 ("Ready to watch · 4 min buffered" / "Playable · may stall
  (finishes in 18 min)"); `playable.acked` set by the acknowledgement rule of §4.5; `wait --until
  playable` keeps the threshold semantics and `wait --until stall-free` is added (exit 8 on
  timeout as for every `--until`); the notification's body click (`default` action) opens the
  player like its "Open" button (§3.8).
- *Why:* A6; §2.7; the product's purpose. Fragments and qBittorrent have nothing like
  `stall_free`; this is where "better than qBittorrent" (§2.7) is earned.
- *Accept:* with the local seed throttled to 1 MiB/s and a 64 MiB `.mkv` fixture, `torrent.playable`
  fires with `prefix_bytes ≥ 32 MiB` before `torrent.completed`; the notification's "open" runs the
  fake `xdg-open` with the file; `wait --until playable` exits 0 before completion; an `.iso`
  fixture never becomes `playable`; a paused torrent keeps its `playable` state; after a daemon
  restart the event does not fire again for an already-playable torrent that is still incomplete
  unless the prefix was lost.
- *Tests:* unit `watch::tests::{prefix_from_bitfield, threshold_rules, largest_video_file,
  stall_free_rule (table over prefix, size, bitrate, dl_bps, logging each row), no_duration_fallback}`;
  `daemon.py::{playable_before_complete, playable_never_for_iso, playable_event_once,
  playable_two_file_mid_piece (the E.3 two-file fixture: the prefix is computed from the video
  file's own first piece, not torrent piece 0; R15),
  wait_until_playable, playable_stall_free_flips (throttle the seed below the bitrate: `ready`
  true, `stall_free` false and the row says "may stall"; raise the throttle: `stall_free` true
  within 2 s), playable_acked_by_open}`; TUI goldens with a Ready row and a "may stall" row;
  `qs-visual.sh popup ready` and `popup may-stall`; `agent_contract.py::{wait_until_playable_exit_codes,
  wait_until_stall_free}`.
- *Depends on:* P5-06, P1-13, P3-10, P4-03, P2-03, P1-18, P2-04 (the bar icon's ready state).

**P5-06 Fork: have-bitfield snapshot accessor** (S)
- *Do:* In `vendor/rqbit/librqbit`, add `ManagedTorrent::have_pieces_snapshot(&self) ->
  Option<Box<dyn BitV>>` (a cloned bitfield) built on `with_chunk_tracker`
  (`torrent_state/mod.rs:266`) and `ChunkTracker::get_have_pieces` (`chunk_tracker.rs:205`),
  returning `None` while initializing or paused without a tracker; document it in
  `vendor/rqbit/README.md`.
- *Why:* both P5-05 and P6-02 need the have-bitfield; the accessor is `pub(crate)` today.
- *Accept:* the accessor returns the bitfield for a live torrent in a fork unit test and `None`
  for a paused one without state; it clones under the lock in under 1 ms for 10,000 pieces.
- *Tests:* fork unit tests `have_pieces_snapshot_{live, paused, timing}`.
- *Depends on:* P0-05, P1-19.

**P5-07 Low disk space warning and `disk_full`** (S)
- *Do:* The supervisor computes `needed = Σ (size − completed_bytes)` over running, incomplete
  torrents and `statvfs` of each distinct `save_path` every 5 s (one syscall per path, no I/O).
  When `free_space < needed + 512 MiB`, `Summary.disk = {free_bytes, needed_bytes, low: true}`
  and every UI shows a banner "Not enough space for the running downloads: 3.1 GiB needed, 1.2 GiB
  free" (urgent tint, §4.2.3; §4.11 state "disk low"); when the engine reports `ENOSPC` the
  torrent's `state` is `error` with `reason: "disk_full"` and the word "Disk full" (§4.3.4). The
  add sheet refuses with "Needs 8.9 GiB · 1.2 GiB free" in urgent when `size > free_space` of the
  chosen destination; `fbittorrent add` prints the same in `warnings[]` and exits 0 (the engine
  may still finish if the owner frees space), and `add --json` carries `disk: {free_bytes,
  needed_bytes}`. A notification "Disk almost full" (`normal`, action `window`) fires once per
  daemon life. `start` on a `disk_full` torrent retries; the supervisor also retries automatically
  when free space rises above `needed + 512 MiB` and logs `disk_full_recovered`.
- *Why:* §6.2 row 37; principle 4 (truthful state) and heuristic 5 (error prevention) at the one
  moment they pay off for a streaming user.
- *Accept:* with a 64 MiB tmpfs as `save_path` and a 100 MiB fixture, the banner appears before
  the first `ENOSPC`, the torrent ends in `error`/`disk_full`, and freeing space (truncating a
  filler file) resumes it without a `start`; the add sheet refuses a 100 MiB fixture against the
  64 MiB destination.
- *Tests:* `daemon.py::{disk_low_banner, disk_full_reason, disk_full_recovers, add_warns_disk}`
  (tmpfs via `unshare -m` or a loop file; skipped with a logged reason when neither is available;
  each logs `free_bytes`/`needed_bytes` per step); unit `supervisor::tests::needed_bytes_sum`;
  TUI golden with the banner and a `! Disk` row; `qs-visual.sh popup disk-low` and `dashboard
  disk-low`.
- *Depends on:* P1-08, P2-03, P3-09, P4-03 (the TUI as an API v1 client renders `Summary.disk`
  and the `! Disk` row; the banner slot already exists in §5.6's layouts), P2-04 (the bar icon's
  urgent state).

**M4 T1 complete (v0.5.0).** *Depends on:* P5-02, P5-03, P5-04, P5-05, P5-07.

#### Phase P6: T2 features

Each item names its §6.3 row and repeats the parameters an implementer needs; tests log as in
§9.1. (Round 3 converted the P6 and P7 tables to this form so that every item carries Do, Why,
Accept, Tests and Depends; the IDs and sizes are unchanged.)

**P6-01 Tracker status and reannounce (fork change)** (L)
- *Do:* §6.3 row "Tracker status and reannounce". **Fork** (`vendor/rqbit/librqbit-tracker-comms/src/tracker_comms.rs`,
  `vendor/rqbit/librqbit/src/torrent_state/mod.rs`): a per-torrent `TrackerStats`
  (`Arc<Mutex<BTreeMap<Url, TrackerStat>>>`) owned by `ManagedTorrentShared` next to its `trackers:
  HashSet<Url>` (`mod.rs:186`), so it survives a pause (comms live in the run region and are
  dropped on pause); `TrackerStat { status, last_announce_at: Option<SystemTime>, next_announce_at:
  Option<SystemTime>, peers: Option<u32>, message: Option<String> }`; passed into
  `TrackerComms::start` from `Session::make_peer_rx` (`session.rs:1378-1440`) and updated at every
  announce result: the HTTP loop's `Ok(interval)`/`Err` at `tracker_comms.rs:255-266` (the backoff
  10 s doubling to 600 s), the UDP path at `:385,400`, the bencoded failure reason at `:314`.
  `ManagedTorrent::tracker_stats() -> Vec<TrackerStat>` (sorted by URL) and
  `ManagedTorrent::announce_now()` (no URL argument: the API never passes one), a `Notify` that
  wakes both the interval sleep and the backoff sleep and sends `event=none`; at most one forced
  announce per tracker per 60 s (later calls are no-ops); DHT is untouched. URLs the fork drops
  (unsupported schemes, `:160-165`; private truncation to the first tracker, `session.rs:1408-1413`)
  appear with `status: not_contacted` and `message: "unsupported scheme"` or `"private torrent:
  only the first tracker is announced"`. The fork unit test lives in `librqbit` (inside CI's
  `engine-libs` command, §8.4). **Daemon** (`src/api/ops/torrents.rs`): `torrents.trackers {hash}`
  → `{trackers: [Tracker]}` (A.9: `url`, `tier?`, `status: ok | error | updating | not_contacted`,
  `last_announce_at?`, `next_announce_at?`, `peers?`, `message?`), sorted by `url` (the Appendix F
  determinism rule); for a torrent that is not live (paused, done, dormant, metadata, parked) it
  returns the recorded stats if any, else every URL as `not_contacted`; `tier` is absent (the
  fork flattens tiers, `session.rs:1021-1031`; tiers return only if the daemon re-reads the
  `.torrent`, not in this item); `message` is capped at 200 characters and every URL in a message
  or a log line has its query string replaced by `?…` before it leaves the fork's error text
  (`rqbit-runtime/src/http.rs:139` echoes the full announce URL, which carries private-tracker
  passkeys; §3.9); `url` itself is served complete to the owner's own clients but the UIs display it
  without the query string. `torrents.reannounce {hashes (1..256)}` → `{state: "done"}` means
  "triggered"; idempotent (A.1); on a torrent that is not live it is `invalid_state`; while
  parked, `engine_unavailable`. The `torrent:<h>.trackers` topic (A.7, keyed by `url`, 5,000 ms): a
  status change is structural and goes out at once (§3.4.7), rates and times ride the tick.
  `TorrentDetail.tracker_count` counts every URL in the torrent's list. `caps` gains `trackers`;
  the §9.3 preamble's contract bookkeeping applies (`api::ops::TABLE` rows; `docs/IPC.md`
  changelog; `contracts/ipc/v1/trackers.ndjson`). **UIs:** the status words and glyphs are four
  rows added to `contracts/ui/states.json` (§4.3.4): `tracker.ok` `✓` "OK", `tracker.error` `!`
  "Error", `tracker.updating` `⟳` "Updating", `tracker.not_contacted` `…` "Not contacted"; columns
  URL (fill, middle-elided, no query string), STATUS 14 cells / `Style.space(120)`, LAST 10 cells /
  `Style.space(80)` as "12 s ago" (`text.rs::ago`; `Model.ago()` added to `Model.js` here), PEERS
  6 cells / `Style.space(56)`; `message` on a second dim line under an `error` row; next announce
  and tier are not shown. Dashboard: the Trackers tab in `DetailsPane.qml` (§4.7), subscribed per
  tab (P5-02's rule), hidden when `caps` lacks `trackers`; `e` reannounces the torrent. TUI:
  `src/tui/modals/trackers.rs` as the Trackers tab of the details modal (P4-04), `e` in that tab
  (`Scope::Modal(Details)`, the Appendix B row "Reannounce"). **CLI:** `fbittorrent reannounce
  <sel>…` (Appendix F): `{ok, action: "reannounce", count, hashes}`, exit codes 0, 1
  (`invalid_state`), 4, 6, 7; `show --json` gains `tracker_count` and `trackers[]`. **Fixture:**
  `fixtures.tracker_server(fail_first=2, interval_s=5, peers=[seed])` in `fixtures.py`: an HTTP
  tracker that answers the first `fail_first` announces of an info hash with a bencoded
  `failure reason: "fixture: not yet"`, then valid responses with `interval: 5` listing the local
  seed, appending `(ts, info_hash, event, port)` to `<tmp>/tracker.log` for every announce.
- *Why:* diagnose stalls (§6.2 rows 2, 9).
- *Accept:* against the fixture, the first announce shows `status: error` with the failure-reason
  text and a `reannounce` (not the 10 s backoff) turns it `ok` with `peers: 1`, in both UIs; a
  reannounce produces an announce in `tracker.log` within 2 s; a second reannounce within 60 s
  produces none; a paused torrent keeps its last recorded status; no query string appears in any
  UI, log line or `message`.
- *Tests:* fork unit test `tracker_stats_records_error_then_ok` (in `librqbit`);
  `daemon.py::{tracker_status (subscribes to the topic and logs each frame), reannounce,
  reannounce_rate_limited, reannounce_not_live_invalid_state, tracker_url_redacted}`; TUI golden
  `details-trackers-120x30.txt` from fixture state `trackers` (the modal renders at 100×30 at that
  width, P4-04); `qs-dashboard.sh::trackers_tab` (`state().view.details.trackers` lists two rows
  with the expected status words); `agent_contract.py::reannounce_verb`.
- *Depends on:* P1-08, P3-10, P4-02, P4-03, P4-04.

**P6-02 Pieces bar** (M)
- *Do:* §6.3 row "Pieces bar": `torrents.pieces {hash}` → `Pieces` (A.9: `count`, `piece_size`,
  `have` as the MSB-first bitfield base64-encoded from P5-06's snapshot, `priority: [u64]` with the
  first and last piece of every included file, `playable_end?` from P5-05's prefix) and the
  `torrent:<h>.pieces` topic (A.7, `state` frames, 2,000 ms); `components/PiecesBar.qml` (one row
  of `Style.space(4)` cells, `Color.foreground` for have, `Util.alpha(foreground, 0.22)` for
  missing, `Color.accent` for priority pieces, the playable prefix underlined) in the dashboard's
  Pieces tab (§4.7); the TUI's Pieces tab as a one-line ftui-extras `canvas` row with `█`, `▒`
  and `▲` (§5.8); `caps` gains `pieces`.
- *Why:* visible proof that sequential and first/last work (§6.2 row 25).
- *Accept:* a 1,000-piece fixture renders with first and last pieces and the playable prefix
  marked in both UIs; a 200,000-piece `Pieces` frame is under 64 KiB.
- *Tests:* unit `pieces::tests::{bitfield_roundtrip, frame_under_64k_200000}`; TUI golden;
  `qs-dashboard.sh::pieces_bar`; fixture `regression_tests/fixtures/states/pieces-1000.json`.
- *Depends on:* P5-06, P1-08, P3-10, P4-03.

**P6-03 Speed history and sparklines** (S)
- *Do:* §6.3 row "Speed history": in the daemon one ring of 600 one-second global samples
  (`download`, `upload`; no per-torrent rings: nothing in §4.7 or §5.6 draws one, and the
  qBittorrent "Speed" tab became the global sparkline, §4.7); `Summary.history: [u64]` (the last 60
  download samples, A.9) and `app.stats {history: true}` → `Stats.history {download, upload,
  interval_s: 1}` (A.4, A.9); the `stats` topic (A.7, 5,000 ms); `status --json` gains `history`
  (Appendix F); `components/Sparkline.qml` in the dashboard's status row (§4.6 "Bottom rows";
  redraw on data, no interpolation, §4.13); ftui `sparkline` in the TUI tab bar (§5.8); `caps`
  gains `history`.
- *Why:* comfort and diagnosis (§6.2 row 10).
- *Accept:* with a local transfer running, `status --json`'s `history` has 60 entries whose last
  ten are non-zero; the sparkline golden shows the ramp of a throttled fixture.
- *Tests:* unit `history::tests::ring_bound_and_order`; TUI sparkline golden at 120×30;
  `qs-smoke.sh::history_points`; `agent_contract.py::status_history_field`.
- *Depends on:* P1-08, P3-09, P4-04.

**P6-04 Statistics, session and all-time** (S)
- *Do:* §6.3 row "Statistics": session totals from the engine (`src/engine.rs:616-623`) and
  all-time totals `{downloaded, uploaded, completed}` persisted in
  `~/.local/state/fbittorrent/stats.json` (0600, temp-fsync-rename), saved on change at most every
  30 s and at drain; `app.stats` → `Stats` (A.4, A.9); `status --json` gains `all_time`
  (Appendix F); a Stats section in the dashboard's Settings sheet and in the TUI's Log modal
  footer ("session ↓ 4.2 GiB ↑ 120 MiB · all time ↓ 1.2 TiB ↑ 41 GiB · 312 completed", `format.rs`
  sizes).
- *Why:* totals are served but never shown (§6.2 row 26).
- *Accept:* totals survive a daemon restart; `status --json` has `all_time`.
- *Tests:* `daemon.py::stats_persist` (download a fixture, restart the daemon, totals unchanged
  plus the new session); unit `stats::tests::save_throttle_30s`.
- *Depends on:* P1-08, P3-15, P4-03.

**P6-05 Force start and queue order** (M)
- *Do:* §6.3 row "Force start and queue order". **Ledger and queue** (`src/engine/ledger.rs`,
  `src/engine/queue.rs` after P1-02; today `Record` at `src/engine.rs:37-53` and `reconcile_queue`
  at `:280-343`): `Record.force: bool` (serde default `false`, next to the existing
  `resume_after_migration` default); the ledger order is the queue order (today's only insert is
  `ledger.records.push`, `src/engine.rs:949`; restored, undone (P6-08) and deferred (P7-09) records
  go to the end; no code path may sort and save `ledger.records`, guarded by
  `ledger::tests::save_preserves_order`); admission is a pure function `queue::admit(records,
  stats, limit) -> Decisions` that `reconcile_queue` applies: forced records always run and do not
  count toward `max_active_downloads` (`src/config.rs:15`, default 3); unforced records with
  intent `run` are admitted in ledger order while unforced running records are fewer than the
  limit; reordering never pauses a running torrent (a moved torrent takes the next slot that
  frees; slots are released only by pause, stop, completion or error); `Torrent.queue_pos` (A.9)
  is the 1-based position among records with intent `run`, `completion_on == 0` and `force ==
  false`, including those in `error`, `metadata`, `waiting_vpn` or without a handle (they are
  waiting too); held records stay `queued` with `reason: queue_full` and the secondary text "N of
  M slots busy · #pos" (§4.3.4); `force` turns off on `torrents.stop`, on completion and on
  `force_start {on: false}`, which puts the record back at its ledger position (it keeps running
  if a slot is free, else `queued`). **API** (`src/api/ops/torrents.rs`): `torrents.force_start
  {hashes (1..256), on}` → `{state: "done"}` (idempotent, A.1): `on: true` also clears a user pause
  (it is an explicit "run now"); on a `done` torrent with seeding off it is `invalid_state` with
  the hint `fbittorrent seed <hash>`; on an `error` torrent it retries like `start`; on a staged
  torrent `invalid_state`; while parked it writes the ledger and applies on unpark (as `start` and
  `stop` do today, `src/engine.rs:1341-1348`). `torrents.queue_move {hash, to: top | up | down |
  bottom}` → `{state: "done", queue_pos}` (not idempotent, A.1; refused with `invalid_state`
  after a restart like every non-idempotent op, P1-09): `up`/`down` swap with the nearest record
  that has a `queue_pos` (paused, done and forced records are skipped), `top`/`bottom` move to the
  first or last position among positioned records; on a torrent without `queue_pos` it is
  `invalid_state`; while parked it reorders the ledger offline. `torrents.start {force: true}` sets
  `force` and lists an already-running torrent in `started`, not `skipped`. `Torrent.force`,
  `Torrent.queue_pos` (A.9, A.10); `caps` gains `queue`; the §9.3 preamble's contract bookkeeping
  applies (`contracts/ipc/v1/queue.ndjson`). **UIs:** the state word stays `Downloading` /
  `Queued`; a forced torrent shows "· forced" in the secondary text and the `⚡` text glyph after
  the state cell in the TUI (a `forced` row in `contracts/ui/states.json`); `queue_pos` shows as
  "#3" in the secondary text of queued rows and as the sort key "Queue" (`s`, Appendix B); `P`
  toggles `force` on the selection (`hashes`) and `K`/`J` move the cursor row (`queue_move
  {to: up|down}`), the cursor following the moved row; the dashboard sets `PanelKeyCatcher`'s
  `reorderable: true` and handles its single `reorderRequested(dy)` signal (`Ui/PanelKeyCatcher.qml:39,42`;
  it fires for Ctrl+k/Ctrl+j and also for Ctrl+↑/Ctrl+↓, `:64-70`, which Appendix B lists); a
  toast "Forced: <name>" / "Queue #3: <name>" (§4.10 feedback); `P`, `K`, `J` are hidden when
  `caps` lacks `queue`. **CLI:** `start --force <sel>…` (`action: "start"`, plus `forced:
  [hash]`) and `queue <sel> top|up|down|bottom` (exactly one torrent: several matches exit 7,
  `all` exits 2; JSON `{ok, action: "queue", hash, queue_pos}`; exit 1 for a torrent without a
  position; Appendix F).
- *Why:* "watch this one now" when the slots are full (§6.2 row 20).
- *Accept:* with `max_active_downloads = 1` and three throttled fixtures (the seed/leech pair of
  P1-07 at 64 KiB/s so nothing completes during the test), force-starting the second runs both
  (both `downloading`/`stalled` with intent `run`, `Summary.counts.downloading == 2`,
  `queue_pos` absent on the forced one); `queue_move {to: top}` on the third gives it `queue_pos:
  1` without pausing the running one, and pausing the running one starts it; `queue_move` on the
  forced one is `invalid_state`; `force_start {on: false}` on the second returns it to the queue.
- *Tests:* unit `queue::tests::{admission_with_force, reorder_positions, up_skips_unpositioned}`
  (over `queue::admit` with synthetic records and stats) and `ledger::tests::save_preserves_order`;
  `daemon.py::{force_start_bypasses_limit, queue_move_order (the three-fixture sequence above, one
  step per call, logging each `queue_pos`), force_off_returns_to_queue}`;
  `agent_contract.py::{start_force, queue_verb, queue_verb_ambiguous_exit_7}`;
  `src/tui/tests/queue.rs::{k_j_send_queue_move, p_toggles_force}` (assert the request the store
  sends through its mock client); `qs-dashboard.sh::{force_start_key, queue_move_keys}`.
- *Depends on:* P1-09, P3-09, P4-02, P4-03.

**P6-06 App-owned fast resume (fork change)** (L)
- *Do:* §6.3 row "Fast resume": in the fork, `initializing.rs` accepts a trusted have-bitfield
  (today it refuses, `vendor/rqbit/librqbit/src/torrent_state/initializing.rs:113-114`); the
  daemon writes `engine/resume/<hash>.bin` (the bitfield plus size and mtime of every included
  file) on clean drain and every 5 minutes for live torrents, and `engine/resume/CLEAN` as the
  clean-shutdown marker; on start a record is restored with the trusted bitfield only when the
  marker existed and every file's size and mtime match, else with a full check (`checked: true`
  after a trusted restore, A.9); `torrents.recheck` deletes the record's resume file first
  (P5-03); `docs/engine-evaluation.md` gains the fast-resume section (§8.5).
- *Why:* the rest of D3.
- *Accept:* restarting with untouched data shows no `checking`; touching one file re-checks only
  that torrent; `kill -9` (no marker) forces a full check of every torrent.
- *Tests:* fork unit test `trusted_bitfield_accepted`; `daemon.py::{resume_trusted,
  resume_touched_file, resume_after_kill9, recheck_drops_resume_file}`.
- *Depends on:* P1-16, P5-03 (recheck deletes the resume file first).

**P6-07 Live per-torrent upload cap without rebuild (fork change)** (S)
- *Do:* §6.3 row "Live upload cap": in the fork, expose the live torrent's `Limits`
  (`vendor/rqbit/librqbit/src/torrent_state/live/mod.rs:217`) through
  `ManagedTorrent::set_upload_bps(Option<u64>)` so a cap change applies without a rebuild (today
  `setUploadLimit` re-adds the torrent and it re-checks); `torrents.set_upload_limit` answers
  `{state: "done"}` synchronously (A.5; `pending` until this item) and `set_seed` uses the same
  path; `seed`/`noseed` stop re-checking.
- *Why:* no surprise re-check on a cap change (§6.2 row 23).
- *Accept:* a cap change never enters `checking`; the new cap is visible in the leecher's rate
  within 5 s.
- *Tests:* fork unit test `live_limits_update`; `daemon.py::cap_change_no_check`.
- *Depends on:* P1-19.

**P6-08 Origin field, filters, bulk remove, undo for keep-files removal** (M)
- *Do:* §6.3 row "Origin, filters, bulk remove, undo": `Record.origin: open(manual | search | rss |
  shows | clipboard | handler | agent)`, serde default `manual` for old records, set by
  `torrents.add {origin}`, `search.grab`, `rss.grab`, `shows.grab`, `add-clipboard` and the desktop
  handler (Appendix F); `torrents.list {origin: [str]?}` filters; the sidebar's "From:" buckets
  (§4.6) are Search, RSS, Shows and Manual (clipboard, handler, agent and unknown values count as
  Manual). Bulk remove is keep-files only: `X` acts on the rows currently listed (the active state
  filter, the `/` text filter and the From filter together, in both UIs), confirms with the count
  and the summed `size` of the rows that have one ("+ 2 of unknown size" otherwise) through
  `ConfirmDialog` (`selectedIndex: 0`) or the TUI prompt (`y/N`), then sends one `torrents.remove
  {hashes (≤ 256), delete_files: false}`; deleting files stays per torrent (`D`) or `delete --files
  --yes`. Undo state is in memory and dies with the daemon: `torrents.remove {delete_files: false}`
  moves each record and its `.torrent` (`src/engine.rs:166`) to `engine/undo/<hash>.{json,torrent}`
  and answers `undo_until` (now + 10 s, A.5); `torrents.add {source: {restore: hash}}` within the
  window re-adds from there, else `invalid_state`; `fbittorrent undo [<hash>]` with no hash
  restores the last batch (`undo_until` is per batch, Appendix F). Toast: the dashboard's `Banner`
  component at the bottom of the list, "Removed 'X' (3 torrents) · U undo", for 10 s; the TUI's
  P4-08 toast with the same text; `U` restores (Appendix B). `caps` gains `origin`.
- *Why:* W8 cleanup; grouping without categories (§6.2 rows 21, 32).
- *Accept:* origin persists across a restart; a filtered bulk removal shows count and size and
  needs `y`; undo within 10 s re-adds and checks to `done` without re-downloading; `undo` after
  the window is `invalid_state` (exit 1).
- *Tests:* `ledger::tests::origin_default_manual`; `daemon.py::undo_restore` (a completed local
  fixture is removed and restored: `done` within 10 s, no re-download; an incomplete one restores
  as `checking` then `downloading`); `daemon.py::bulk_remove_mixed_sizes`;
  `qs-dashboard.sh::{bulk_remove_confirm (the dialog text names count and size; Enter cancels),
  undo_toast (`state().toast` names the batch; `U` restores; the toast is gone at 11 s)}`;
  `agent_contract.py::undo_verb`.
- *Depends on:* P3-09, P4-04, P4-08.

**P6-09 Peer port setting and sleep inhibitor** (S)
- *Do:* §6.3 row "Port setting and sleep inhibitor": `Settings.peer_port` (A.9; a restart-class
  key, applied through `app.settings.set {peer_port}` with the engine restart of P3-05 while
  clients stay connected), a field in the dashboard's Settings sheet and the TUI's F6 modal, and
  `settings --port N` (Appendix F); `src/daemon/inhibit.rs` (§3.8 "Sleep inhibitor"): while any
  torrent is `downloading`, `stalled`, `metadata` or `checking` with intent `run`, the daemon holds
  one child `systemd-inhibit --what=sleep --who=fbittorrent --why=Downloading --mode=block sleep
  infinity` and kills it within 5 s of the last such torrent stopping; the child is also killed on
  drain.
- *Why:* §6.2 rows 27, 36.
- *Accept:* a port change restarts the engine with clients still connected (they receive
  `engine.changed`); the inhibitor child exists only while a download runs.
- *Tests:* `daemon.py::{port_restart_keeps_clients, inhibitor_lifecycle (a `systemd-inhibit`
  shim records start and SIGTERM times)}`.
- *Depends on:* P3-05, P3-15, P4-03.

**P6-10 New-release notifications for Shows** (S)
- *Do:* §6.3 row "Shows notifications": after a `shows.check` (scheduled or explicit) that found new
  unread releases, one notification per check through P1-13's notifier: title "3 new episodes",
  body "Slow Horses S05E03, South Park S27E06, …" (at most three names, then "+ N more"), action
  `window` → `fbittorrent dashboard --view shows` (§3.8 row "new releases"); never for a check that
  found nothing; suppressed while a dashboard or TUI is interactive (the badge shows it instead).
- *Why:* closes the follow-a-show loop (W2 step 2).
- *Accept:* a check against the EZTV fixture with new releases sends one notification; a second
  check with nothing new sends none.
- *Tests:* `daemon.py::{shows_notify, shows_notify_none_when_unchanged, shows_notify_suppressed_interactive}`.
- *Depends on:* P3-03, P1-13.

**P6-11 Destination picker with free space in the add sheet** (S)
- *Do:* §6.3 row "Add sheet destination": `app.free_space {path}` → `{free_bytes?, writable}`
  (A.4; `statvfs`; absent on error, never negative, §3.4.9); in `AddSheet.qml` a destination row
  showing the free space ("312 GiB free") and a folder picker (`components/FolderPicker.qml`: a
  small directory browser over `Quickshell.Io`'s folder listing, because Quickshell has no file
  dialog [FO §2.11]; starts at `save_path`, Enter selects, `..` goes up, hidden folders skipped);
  the sheet refuses a destination that is not writable; `add --save <dir>` validates the same way
  (exit 2 when not writable).
- *Why:* avoid filling the disk; Quickshell has no file dialog [FO §2.11].
- *Accept:* free space is absent on error and never negative (unit); the picker selects a folder
  and the committed torrent lands there; an unwritable folder is refused in the sheet and by the
  CLI.
- *Tests:* unit `free_space::tests::{absent_on_error, never_negative}`;
  `qs-dashboard.sh::destination_picker`; `agent_contract.py::add_save_unwritable`.
- *Depends on:* P3-11.

**P6-12 Private-torrent rules (BEP 27) check** (S)
- *Do:* §6.3 row "Private flag": verify that the fork skips DHT and PeX for torrents with
  `private=1` (upstream threads `info.private` into the session,
  `vendor/rqbit/librqbit/src/session.rs:1115,1125,1365-1386`, `is_private`); add a fork unit test
  with a private fixture torrent and a local tracker asserting no DHT announce and no PeX message
  is sent; `TorrentDetail.private` (A.9) shown in both Overview views as "private (no DHT, no
  PeX)"; fix the fork if the test fails.
- *Why:* private trackers must not leak through DHT or PeX; no app test covers the flag today.
- *Accept:* a private fixture torrent produces no DHT queries and no PeX in the engine log.
- *Tests:* fork test `private_no_dht_no_pex`; `daemon.py::private_flag_shown`.
- *Depends on:* P0-05, P1-07.

**P6-13 Clipboard magnet watch (opt-in, A7)** (M)
- *Do:* §6.3 row "Clipboard watch": `Settings.clipboard_watch` (default `false`); when on, the
  daemon (`src/daemon/clipboard.rs`) runs one child `wl-paste --watch cat` with bounded output
  (lines over 64 KiB dropped), parses magnets from its stream, dedupes by btih against the ledger
  and the last offer, and emits `clipboard.magnet {hash, name?, already_added}` (A.8) plus the
  notification "Magnet on clipboard: <name>" with actions `add` → `torrents.add {origin:
  "clipboard"}` and `dismiss` → `app.clipboard.dismiss {hash}` (§3.8); the popup offer strip and
  the dashboard banner (§4.11 row "Clipboard magnet") with [Add] [Dismiss]; the TUI footer strip
  "Add '<name>'? (y/Esc)"; turning the setting off kills the child; the child is never a reason
  to stay alive (§3.2.10 `clipboard_watch`); clipboard text is never logged (§3.9); `settings
  --clipboard-watch on|off` (Appendix F); `caps` gains `clipboard`.
- *Why:* Fragments' best idea [FO §5]; §6.2 row 31.
- *Accept:* off by default (`settings` shows it); when on, a fake `wl-paste` printing a magnet
  produces one `clipboard.magnet` event and one notification; "add" adds it; the same magnet is
  not re-offered; turning it off kills the child; the popup strip and dashboard banner appear;
  nothing from the clipboard appears in the log.
- *Tests:* `daemon.py::{clipboard_watch_offer, clipboard_watch_add, clipboard_watch_dedupe,
  clipboard_watch_off_kills_child, clipboard_never_logged}`; `qs-visual.sh popup clipboard` and
  `dashboard clipboard`; TUI golden of the strip.
- *Depends on:* P1-13, P2-03, P3-05.

**M5 T2 complete (v0.6.0).** *Depends on:* P6-01 to P6-13.

#### Phase P7: T3 features

**P7-01 Torznab indexers (§7.3)** (M)
- *Do:* §6.3 row "Torznab": a native provider kind `torznab` in `src/search.rs` (`Provider.kind:
  torznab`, A.9) configured by `search.install {sources: [https URL]}` with the key read from a
  0600 file opened with `O_NOFOLLOW` (§3.9; refused, not repaired, when its mode is wrong); queries
  `?t=search&q=<query>&apikey=<key>` with a 30 s timeout and a 4 MiB response cap; results map to
  `SearchResult` (`magnet` or `url`, `size`, `seeds`, `uploaded_at`); a key over plain HTTP to a
  non-loopback host is refused with `invalid_args`; a cross-origin redirect is refused with
  `upstream`; the key never appears in logs or error messages; the TUI providers modal and the
  dashboard providers sheet list the kind; CLI `search --install-torznab <url> --key-file <path>`
  (Appendix F); `caps` gains `torznab`.
- *Why:* private trackers without Python (§7.3).
- *Accept:* a Torznab fixture server returns results that appear deduplicated with the other
  providers; a key over plain HTTP to a non-loopback host is refused; a cross-origin redirect is
  refused; the key never appears in logs.
- *Tests:* `daemon.py::torznab_{results, http_key_refused, redirect_refused, key_not_logged}`
  (`fixtures.torznab_server()` with a redirect case); unit `torznab::tests::parse_fixture`.
- *Depends on:* P3-01.

**P7-02 Move / set location** (M)
- *Do:* §6.3 row "Move": `torrents.move {hashes, save_path}` → `{state: "pending"}` then
  `mutation.result` (A.5; not idempotent, A.1): pause, move the payload (rename on the same
  filesystem; copy, fsync, unlink across filesystems), update `Record.output_folder`, restore with
  fast resume (P6-06) so a completed torrent stays `done`; `Torrent.save_path` follows; a details
  action in the dashboard (Overview "Move…" with the folder picker of P6-11) and `fbittorrent move
  <sel> <path>` (Appendix F); none in the TUI (T3).
- *Why:* library organisation.
- *Accept:* moves within and across filesystems in temporary directories; a completed torrent stays
  `done` without a re-check; a failed copy leaves the source intact and reports `failed`.
- *Tests:* `daemon.py::move_{same_fs, cross_fs, failed_copy_keeps_source}` (cross-filesystem
  through a tmpfs when available, else skipped with a logged reason); `agent_contract.py::move_verb`.
- *Depends on:* P6-06, P3-10.

**P7-03 Rename display name** (S)
- *Do:* §6.3 row "Rename display name": add `Record.display_name: Option<String>` (serde default
  `None`; `Torrent.name` serves `display_name` when set, else `name`; `TorrentDetail.original_name`
  carries the torrent's own name). The restore path and the metadata-resolution path
  (`src/engine.rs:934`, which today overwrites `record.name`) write `record.name` only and never
  touch `display_name`. `torrents.rename {hash, name}` (A.5): `name` is trimmed, 1..512 chars, no
  control characters; an empty string clears the override (revert); the ledger is written with the
  §3.9 temp-fsync-rename rule and a `torrents` delta `{name}` follows. CLI `rename <sel> <name>`
  (Appendix F) starts the daemon and refuses an ambiguous selector with exit 7. TUI: `N` on a
  torrent row opens the prompt "Rename to:" prefilled with the current name; dashboard: the
  Overview title is an inline `TextField` on `N` or double-click. `caps` gains `rename`.
- *Why:* cosmetic; the torrent's own name is often a release string.
- *Accept:* the override survives a daemon restart and a metadata re-resolution; an empty name
  reverts; control characters are refused.
- *Tests:* `ledger::tests::display_name_survives_restore_and_metadata` (rename, then simulate a
  daemon restart by reloading the ledger and re-running the metadata path; the override stands),
  `rename_validation` (table: empty clears, whitespace trimmed, control characters refused with
  `invalid_args` key `name`), `agent_contract.py::rename_verb`.
- *Depends on:* P3-10, P4-03.

**P7-04 Tracker add and remove (fork change)** (M)
- *Do:* §6.3 row "Tracker add/remove": in the fork, a mutable per-torrent tracker set
  (`ManagedTorrent::add_tracker(url)`, `remove_tracker(url)`) persisted in the record
  (`Record.trackers: Vec<String>` merged with the `.torrent`'s list on restore); `torrents.add_tracker
  {hash, url}` and `torrents.remove_tracker {hash, url}` → `{state: "done"}` (A.5; a URL must be
  `http(s)://` or `udp://`, else `invalid_args`); the Trackers tab gains `+` (prompt) and `x`
  (confirm, Cancel focused) in both UIs; reachable from the CLI through `api`; `caps` gains
  `tracker_edit`.
- *Why:* completeness of the trackers view.
- *Accept:* an added tracker receives announces in the fixture; a removed one receives none after
  the next announce cycle.
- *Tests:* `daemon.py::tracker_add_remove` (two local tracker fixtures); fork unit test
  `tracker_set_mutation`.
- *Depends on:* P6-01.

**P7-05 Quick-add and search overlay palette (separate overlay plugin id)** (M)
- *Do:* §6.3 row "Quick-add palette": a second plugin `plugin/oliver.torrents-quick/` with
  `kinds: ["overlay"]` (an overlay kind must not share the bar-widget plugin [OR §6 gotcha 4]),
  summoned by its own chord: one `FilterInput`; a magnet or `.torrent` path pastes → `torrents.add`
  (immediate, with the §3.8 "added" notification); any other text → `search.start` and the first
  ten results as rows; Enter on a result stages it and opens the dashboard's add sheet on it; Esc
  closes; it uses the same `Service.qml` singleton through `serviceFor("oliver.torrents")`.
- *Why:* A3's deferred overlay.
- *Accept:* summon, type a magnet, Enter adds; type a query, results appear, Enter stages; the bar
  plugin's toggle routing is unaffected.
- *Tests:* `qs-smoke.sh::palette`; `qs-dashboard.sh::palette_summon_routing`.
- *Depends on:* P3-12.

**P7-06 Per-torrent download cap** (S)
- *Do:* §6.3 row "Per-torrent download cap": `Record.download_limit: Option<u64>` (bytes per
  second, persisted; absent = unlimited) applied through the live `Limits` P6-07 exposes, so a
  change never re-checks. `torrents.set_download_limit {hashes, bps?}`: `bps` ≥ 16384 or `null`
  for unlimited (the same floor as `torrents.set_upload_limit`, `src/engine.rs:63-65`), idempotent
  (A.1), `{state: "done"}`. Effective rate is `min(per-torrent, global, alt when on)`. `caps` gains
  `download_limit`. CLI `limit <sel> --down N` accepts bytes per second with `K`/`M` suffixes
  (`64K`), `--down none` clears, JSON `{ok, action: "limit", hashes, download_limit_bps}`; exit 2
  for a value below 16 KiB/s. Dashboard: a field in the Overview tab; TUI: none (T3).
- *Why:* rarely needed beside global limits; completes the symmetry with the live upload cap.
- *Accept:* a local transfer stays within the cap's tolerance; a cap change mid-transfer never
  enters `checking`.
- *Tests:* `daemon.py::download_cap_holds` (the P5-04 fixture: local seed, 64 KiB/s cap, after a
  5 s warm-up the leecher's `dl_bps` from `torrents.get` stays within ±25 % over 20 s; a cap change
  mid-transfer never enters `checking`); `limits::tests::effective_rate_is_min`;
  `agent_contract.py::limit_verb`.
- *Depends on:* P6-07, P3-10, P1-11.

**P7-09 Deferred add while waiting for the VPN** (S)
- *Do:* §6.3 row "Deferred add while parked": while the slot is `Parked`, `torrents.add` with a
  magnet saves a paused record from the magnet's btih and `dn=` name and answers `{state: "done",
  hash, deferred: true}` (A.5) instead of `engine_unavailable` (today it fails,
  `src/engine.rs:830-832`); when the engine unparks, the record's metadata resolves and it joins the
  queue with its saved intent; `.torrent` paths are added the same way (metadata already known);
  `{url}` sources stay refused while parked (no network through the tunnel yet); the rows show
  `waiting_vpn`; `add` prints `deferred: true` (Appendix F); `caps` gains `deferred_add`.
- *Why:* an add while parked fails today (`src/engine.rs:830-832`).
- *Accept:* with a missing interface, `add` answers `deferred`; when the interface appears, metadata
  resolves and the torrent starts.
- *Tests:* `daemon.py::deferred_add` with the namespace fixture (`vpn.py`, opt-in `FBT_TEST_VPN=1`;
  the non-namespace part, `deferred_add_saved_record`, runs always).
- *Depends on:* P1-09.

Superseded items keep their IDs:

| ID | Title | Why | Accept (and how it is tested) | Size | Depends on |
|---|---|---|---|---|---|
| P7-07 | TUI retirement review | **Superseded (round 1): removed as ceremony.** A review meeting produces no capability; the owner decided to keep the TUI (A2) and can read the per-kind counters of P1-14 at any time (the `clients` object of `fbittorrent api app.info --json`). | - | - | - |
| P7-08 | Marketplace readiness | **Superseded (round 3): removed as ceremony.** The marketplace is not a goal (DL-17, §2.1); §7.4 keeps a two-line note of what a listing would take, and nothing is designed until the owner asks. | - | - | - |

**M6 T3 complete.** *Depends on:* P7-01 to P7-06, P7-09.

### 9.4 Dependency graph

Every edge is listed in the items above; this table is the same graph as an adjacency list so a
reviewer can check it is acyclic and has no orphans without reading the items.

| Item | Depends on | Consumed by |
|---|---|---|
| P0-01 | — | P0-02, P0-04 |
| P0-02 | P0-01 | P0-04 |
| P0-03 | — | P0-04 |
| P0-04 | P0-01, P0-02, P0-03 | P0-05, P0-07, P0-08, P0-09, P0-10, P0-11, P0-12, P0-13, P0-14, P0-15, P0-16 |
| P0-05 | P0-04 | P0-06, P5-06, P6-12 |
| P0-06 | P0-05, P0-15 | M0 |
| P0-07 | P0-04, P0-15 | M0 |
| P0-08 | P0-04, P0-15 | M0 |
| P0-09 | P0-04, P0-15 | M0 |
| P0-10 | P0-04, P0-16 | M0 |
| P0-11 | P0-04, P0-15, P0-16 | M0 |
| P0-12 | P0-04 | M0 |
| P0-13 | P0-04, P0-16 | M0 |
| P0-14 | P0-04, P0-15 | M0 |
| P0-15 | P0-04 | P0-06, P0-07, P0-08, P0-09, P0-11, P0-14, M0, P1-07 |
| P0-16 | P0-04 | P0-10, P0-11, P0-13, M0, P4-01 |
| M0 | P0-06, P0-07, P0-08, P0-09, P0-10, P0-11, P0-12, P0-13, P0-14, P0-15, P0-16 | P1-01 |
| P1-01 | M0 | P1-02 |
| P1-02 | P1-01 | P1-03, P1-19 |
| P1-03 | P1-02, P1-07 | P1-04, P1-06, P1-10 |
| P1-04 | P1-03 | P1-11, P1-15, P1-17, M1a, P2-07 |
| P1-05 | P1-19 | P1-06, P2-02 |
| P1-06 | P1-03, P1-05, P1-19 | P1-08, P1-09, P3-04, P3-05 |
| P1-07 | P0-15 | P1-03, P1-08, P1-09, P1-16, P2-06, P2-08, P4-10, P6-12 |
| P1-08 | P1-06, P1-07 | P1-12, P1-13, P1-14, P1-18, P2-01, P3-01, P3-02, P3-03, P3-05, P5-01, P5-07, P6-01, P6-02, P6-03, P6-04 |
| P1-09 | P1-06, P1-07 | P1-11, P2-01, P3-06, P5-03, P6-05, P7-09 |
| P1-10 | P1-03, P1-19 | P1-15 |
| P1-11 | P1-04, P1-09, P1-12, P1-14, P1-17 | P1-15, P1-18, M1a, P2-07, P3-07, P7-06 |
| P1-12 | P1-08 | P1-11, P1-15, P1-17, M1a, P2-03, P3-02, P3-03, P3-06 |
| P1-13 | P1-08 | M1a, P5-05, P6-10, P6-13 |
| P1-14 | P1-08 | P1-11, M1, P3-10, P3-15, P4-08 |
| P1-15 | P1-04, P1-10, P1-11, P1-12 | M1a, P3-07, P4-01 |
| P1-16 | P1-07, P1-19 | M1a, P5-03, P6-06 |
| P1-17 | P1-04, P1-12 | P1-11, M1a |
| P1-18 | P1-08, P1-11 | M1a, P5-05 |
| P1-19 | P1-02 | P1-05, P1-06, P1-10, P1-16, P5-06, P6-07 |
| M1a | P1-04, P1-11, P1-12, P1-13, P1-15, P1-16, P1-17, P1-18 | M1 |
| P2-01 | P1-08, P1-09 | P2-02, P2-08, P3-08, P4-09 |
| P2-02 | P1-05, P2-01 | P2-03, P2-04, P2-06, P3-09 |
| P2-03 | P1-12, P2-02, P2-08 | P2-05, M1, P5-04, P5-05, P5-07, P6-13 |
| P2-04 | P2-02, P2-08 | P2-05, P5-05, P5-07 |
| P2-05 | P2-03, P2-04 | M1 |
| P2-06 | P1-07, P2-02 | M1, P3-16 |
| P2-07 | P1-04, P1-11 | M1 |
| P2-08 | P1-07, P2-01 | P2-03, P2-04, P3-09, P4-05 |
| M1 | P1-14, M1a, P2-03, P2-05, P2-06, P2-07 | P4-09 |
| P3-01 | P1-08 | P3-07, P3-12, P4-07, P7-01 |
| P3-02 | P1-08, P1-12 | P3-07, P3-14 |
| P3-03 | P1-08, P1-12 | P3-07, P3-13, P6-10 |
| P3-04 | P1-06 | P3-07, P3-10, P3-12, P3-17 |
| P3-05 | P1-06, P1-08 | P3-11, P3-15, P5-04, P6-09, P6-13 |
| P3-06 | P1-09, P1-12 | P3-11, P3-12, P3-13, P3-14 |
| P3-07 | P1-11, P1-15, P3-01, P3-02, P3-03, P3-04 | P3-18, M2, P4-03 |
| P3-08 | P2-01 | P3-09, P3-12, P3-13, P3-14, P3-15 |
| P3-09 | P2-02, P2-08, P3-08 | P3-10, P3-11, P5-04, P5-07, P6-03, P6-05, P6-08 |
| P3-10 | P1-14, P3-04, P3-09 | P3-16, P5-02, P5-03, P5-05, P6-01, P6-02, P7-02, P7-03, P7-06 |
| P3-11 | P3-05, P3-06, P3-09 | P3-13, P3-16, P6-11 |
| P3-12 | P3-01, P3-04, P3-06, P3-08 | P3-16, P3-17, P7-05 |
| P3-13 | P3-03, P3-06, P3-08, P3-11 | P3-16, P3-18 |
| P3-14 | P3-02, P3-06, P3-08 | P3-16 |
| P3-15 | P1-14, P3-05, P3-08 | P3-16, P6-04, P6-09 |
| P3-16 | P2-06, P3-10, P3-11, P3-12, P3-13, P3-14, P3-15, P3-17, P3-18 | M2 |
| P3-17 | P3-04, P3-12 | P3-16 |
| P3-18 | P3-07, P3-13 | P3-16, P4-07 |
| M2 | P3-07, P3-16 | — |
| P4-01 | P0-16, P1-15 | P4-02, P4-03, P4-05, P4-06 |
| P4-02 | P4-01 | P4-04, P5-03, P5-04, P6-01, P6-05 |
| P4-03 | P3-07, P4-01 | P4-04, P4-08, P4-09, P5-02, P5-03, P5-04, P5-05, P5-07, P6-01, P6-02, P6-04, P6-05, P6-09, P7-03 |
| P4-04 | P4-02, P4-03 | P4-07, P4-10, P4-11, P6-01, P6-03, P6-08 |
| P4-05 | P2-08, P4-01 | M3 |
| P4-06 | P4-01 | M3 |
| P4-07 | P3-01, P3-18, P4-04 | M3 |
| P4-08 | P1-14, P4-03 | M3, P6-08 |
| P4-09 | P2-01, M1, P4-03 | M3 |
| P4-10 | P1-07, P4-04 | M3 |
| P4-11 | P4-04 | M3 |
| M3 | P4-05, P4-06, P4-07, P4-08, P4-09, P4-10, P4-11 | — |
| P5-01 | P1-08 | P5-02 |
| P5-02 | P3-10, P4-03, P5-01 | M4 |
| P5-03 | P1-09, P1-16, P3-10, P4-02, P4-03 | M4, P6-06 |
| P5-04 | P2-03, P3-05, P3-09, P4-02, P4-03 | M4 |
| P5-05 | P1-13, P1-18, P2-03, P2-04, P3-10, P4-03, P5-06 | M4 |
| P5-06 | P0-05, P1-19 | P5-05, P6-02 |
| P5-07 | P1-08, P2-03, P2-04, P3-09, P4-03 | M4 |
| M4 | P5-02, P5-03, P5-04, P5-05, P5-07 | — |
| P6-01 | P1-08, P3-10, P4-02, P4-03, P4-04 | M5, P7-04 |
| P6-02 | P1-08, P3-10, P4-03, P5-06 | M5 |
| P6-03 | P1-08, P3-09, P4-04 | M5 |
| P6-04 | P1-08, P3-15, P4-03 | M5 |
| P6-05 | P1-09, P3-09, P4-02, P4-03 | M5 |
| P6-06 | P1-16, P5-03 | M5, P7-02 |
| P6-07 | P1-19 | M5, P7-06 |
| P6-08 | P3-09, P4-04, P4-08 | M5 |
| P6-09 | P3-05, P3-15, P4-03 | M5 |
| P6-10 | P1-13, P3-03 | M5 |
| P6-11 | P3-11 | M5 |
| P6-12 | P0-05, P1-07 | M5 |
| P6-13 | P1-13, P2-03, P3-05 | M5 |
| M5 | P6-01, P6-02, P6-03, P6-04, P6-05, P6-06, P6-07, P6-08, P6-09, P6-10, P6-11, P6-12, P6-13 | — |
| P7-01 | P3-01 | M6 |
| P7-02 | P3-10, P6-06 | M6 |
| P7-03 | P3-10, P4-03 | M6 |
| P7-04 | P6-01 | M6 |
| P7-05 | P3-12 | M6 |
| P7-06 | P1-11, P3-10, P6-07 | M6 |
| P7-09 | P1-09 | M6 |
| M6 | P7-01, P7-02, P7-03, P7-04, P7-05, P7-06, P7-09 | — |

Topological levels (an item depends only on items in earlier levels; recomputed for round 4 with
`scripts/dagcheck.py`, Appendix E, after every edit to §9.3; 22 levels, 107 nodes (99 items and 8 milestones), no cycle, no orphan):

| Level | Items |
|---|---|
| 0 | P0-01, P0-03 |
| 1 | P0-02 |
| 2 | P0-04 |
| 3 | P0-05, P0-12, P0-15, P0-16 |
| 4 | P0-06, P0-07, P0-08, P0-09, P0-10, P0-11, P0-13, P0-14, P1-07 |
| 5 | M0, P6-12 |
| 6 | P1-01 |
| 7 | P1-02 |
| 8 | P1-03, P1-19 |
| 9 | P1-04, P1-05, P1-10, P1-16, P5-06, P6-07 |
| 10 | P1-06 |
| 11 | P1-08, P1-09, P3-04 |
| 12 | P1-12, P1-13, P1-14, P2-01, P3-01, P3-05, P5-01, P7-09 |
| 13 | P1-17, P2-02, P2-08, P3-02, P3-03, P3-06, P3-08, P7-01 |
| 14 | P1-11, P2-03, P2-04, P2-06, P3-09, P3-12, P3-14, P3-15, P6-10 |
| 15 | P1-15, P1-18, P2-05, P2-07, P3-10, P3-11, P3-17, P6-13, P7-05 |
| 16 | M1a, P3-07, P3-13, P4-01, P6-11, P7-06 |
| 17 | M1, P3-18, P4-02, P4-03, P4-05, P4-06 |
| 18 | P3-16, P4-04, P4-08, P4-09, P5-02, P5-03, P5-04, P5-05, P5-07, P6-02, P6-04, P6-05, P6-09, P7-03 |
| 19 | M2, M4, P4-07, P4-10, P4-11, P6-01, P6-03, P6-06, P6-08 |
| 20 | M3, M5, P7-02, P7-04 |
| 21 | M6 |

Checks: no cycle (every edge goes from a lower level to a higher one); no orphan (every item is
consumed by a milestone or by another item; P7-07 and P7-08 are superseded and outside the graph).

Critical path to the first demo as `scripts/dagcheck.py` prints it (round 4): 18 nodes deep, of
which 15 are items and 3 milestones; weight 32 in S-units with S=1, M=2, L=4. Everything else at M1
can run in parallel with it. The path is not unique: 3 paths to M1 share this weight (the script
prints the first in its dependency order), because the P0 prefixes P0-01 → P0-02 → P0-04 → P0-05 →
P0-06 and P0-01 → P0-02 → P0-04 → P0-15 → P0-06 and P0-01 → P0-02 → P0-04 → P0-15 → P0-09 tie, and
after P1-08 the daemon branch (P1-12 → P1-17 → P1-11 → P1-15 → M1a) and the plugin branch (P2-01 →
P2-02 → P2-03 → P2-05) both weigh 9. The ties below list every path of maximal weight to M1a and M1.

```
P0-01 ─► P0-02 ─► P0-04 ─► P0-05 ─► P0-06 ─► M0 ─► P1-01 ─► P1-02 ─► P1-19 ─► P1-05 ─► P1-06 ─┐
  └─► P1-08 ─► P1-12 ─► P1-17 ─► P1-11 ─► P1-15 ─► M1a ─► M1
```

Weighted longest paths (the script's output, round 4):

```
M1a: 17 nodes (15 items), weight 32: M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M1: 18 nodes (15 items), weight 32: M1 ← M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M2: 20 nodes (18 items), weight 38: M2 ← P3-16 ← P3-18 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M3: 21 nodes (19 items), weight 46: M3 ← P4-07 ← P4-04 ← P4-03 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M4: 20 nodes (18 items), weight 42: M4 ← P5-02 ← P4-03 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M5: 21 nodes (19 items), weight 48: M5 ← P6-01 ← P4-04 ← P4-03 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
M6: 22 nodes (20 items), weight 50: M6 ← P7-04 ← P6-01 ← P4-04 ← P4-03 ← P3-07 ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01
```

Every path of maximal weight (ties):

```
M1a: M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01 | M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-15 ← P0-04 ← P0-02 ← P0-01 | M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-09 ← P0-15 ← P0-04 ← P0-02 ← P0-01
M1: M1 ← M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-02 ← P0-01 | M1 ← M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-06 ← P0-15 ← P0-04 ← P0-02 ← P0-01 | M1 ← M1a ← P1-15 ← P1-11 ← P1-17 ← P1-12 ← P1-08 ← P1-06 ← P1-05 ← P1-19 ← P1-02 ← P1-01 ← M0 ← P0-09 ← P0-15 ← P0-04 ← P0-02 ← P0-01
```

Sizes: L 13, M 44, S 42 (99 items; milestones and superseded items excluded).

Parallel lanes (items in different lanes touch mostly different files):

| Lane | Files | Items |
|---|---|---|
| Daemon and API | `src/daemon/`, `src/api/`, `src/engine/`, `vendor/rqbit` | P1-01 to P1-14, P1-16 to P1-19, P3-01 to P3-07, P5-01, P5-03 to P5-07 (daemon side), P6-01, P6-04 to P6-07, P6-09, P6-10, P6-12, P6-13, P7-01, P7-02, P7-04, P7-06, P7-09 |
| Plugin | `plugin/oliver.torrents/`, `plugin/tests/` | P2-01 to P2-08, P3-08 to P3-18, P5-02 (QML side), P6-02, P6-03, P6-08, P6-11 (QML sides), P7-05 |
| TUI | `src/tui/` | P0-10, P0-11, P0-13, P0-16, P1-15, P4-01 to P4-11, P5-02 (TUI side), P6-03 and P6-08 (TUI sides) |
| Hygiene and harness | repo root, `docs/`, CI, `scripts/`, `regression_tests/` | P0-01 to P0-06, P0-12, P0-15, P1-07 |

### 9.5 Definition of done per phase

A phase is done when every item's acceptance criteria pass **and** the phase's scripts below are
green on `main`. Nothing else (no sign-off meeting, no report).

| Phase | Scripts that must be green | Human check (one, concrete) |
|---|---|---|
| P0 | `all.sh` (90+ tests, `native.py` with JSONL logs, goldens, `fbtlog_test.py`); `cargo test -p librqbit-core -p librqbit`; CI on `main` | The owner installs `v0.1.2`, closes the Ghostty window during a download and sees no stale socket |
| P1 | `all.sh` + `daemon.py` (every P1 case) + `agent_contract.py` + `native.py` (the TUI as a client); `resources.py --daemon` baseline written | The owner clicks a magnet link with no window open and gets a notification, not a terminal |
| P2 | P1's set + `qs-store.sh` + `qs-smoke.sh` + `plugin/tests/lint.sh` + `contrast.py` + `widget_keys.sh` + `qs-visual.sh` captures stored | The owner opens the popup in their theme and in `catppuccin-latte` and finds every state readable |
| P3 | P2's set + `qs-dashboard.sh` (every case, `width_tiers` at the four tier boundaries) + `daemon.py` P3 cases + captures at 900/1280/1500/1600 | The owner runs the M2 demo script (§9.7) without a hitch |
| P4 | P3's set + `tui_smoke.py` at three sizes + `palette` tests + `agent_contract.py::list_human_matches_tui_format` | The owner uses the TUI at 90 columns over SSH and sees state words on every row |
| P5 | P4's set + `daemon.py` P5 cases (including `playable_before_complete`, `playable_stall_free_flips`, `disk_low_banner`) + fork unit tests | The owner starts a film and gets "Ready to watch" before it finishes, and a "may stall" row when the turtle limit is below the bitrate |
| P6 | P5's set + `daemon.py` P6 cases + `FBT_TEST_RESOURCES=1` gate | The owner turns on clipboard watching for a day and reports whether it is useful |
| P7 | P6's set + the P7 cases | n/a |

### 9.6 Verification scripts

The complete list, with what each covers and when it runs, is Appendix E. In one line each:

`all.sh` (umbrella, flags for the opt-in suites, summary table) · `cargo test` (unit; goldens;
contract round-trips; palette; action table) · `native.py` (isolated TUI/CLI, on `fbtlog.py`) ·
`daemon.py` (every daemon behaviour; `--serve-fixture` for visuals) · `agent_contract.py` (CLI
contract: exit codes, envelopes, determinism, stdout/stderr split, events, api) ·
`fbtlog_test.py` (the logging contract) · `fixtures.py` (payloads, servers, shims) · `qs-log.py`
(QML marker lines to JSONL) · `qs-store.sh` (Store fixtures) ·
`qs-format.sh` (number formatting) · `qs-glyphs.sh` (glyph rendering) · `qs-smoke.sh` (plugin to
daemon) · `qs-dashboard.sh` (dashboard behaviour on the live shell) · `qs-visual.sh` (captures and
pixel diffs) · `widget_keys.sh` (popup keys) · `tui_smoke.py` (tmux at three sizes) ·
`plugin/tests/lint.sh` (tokens, structure) · `contrast.py` (theme contrast) · `vpn.py` (namespace
fixtures) · `resources.py` (budget gate) · `scripts/check-doc-links.sh` (run by hand when receipts move),
`scripts/lint-sizes.sh`, `scripts/verify-determinism.sh`, `scripts/gen-keymap.sh` (lints and
generators) · `scripts/dagcheck.py` (this plan's §9.3
against §9.4: undefined IDs, cycles, orphans, consumed-by consistency, the level table, weighted
critical paths) · `scripts/mockwidth.py` (every mockup line in §5 and Appendix C measured in cells
with East Asian width rules and Nerd Font PUA = 1; overflow and ragged boxes fail). Both ran clean
on this round; they are review-round tools, run by hand before each round and before the beads
conversion, not CI jobs: the plan stops changing once it is beads.

### 9.7 Milestones and demo scripts

| Milestone | What the owner can do | Release |
|---|---|---|
| M0 | Use today's app, now in git, without the relaunch bug, the yes-by-default deletes or the startup blocker | v0.1.2 |
| M1a | Magnet clicks without terminals; downloads survive closing the TUI; idle exit with `fbittorrent status` showing the countdown; completion and added notifications; `events`/`api` for agents; the old popup still works through the new CLI (`status --json`, `--no-start` actions) | v0.2.0-pre (installed) |
| **M1** | Magnet clicks without terminals; live bar popup in the §4 design; downloads survive closing windows; idle exit; completion and added notifications; the TUI as a client; `events`/`api` for agents | **v0.2.0** |
| M2 | Run everything from the dashboard window, including the poster grid and show quality | v0.3.0 |
| M3 | A responsive, readable TUI on any terminal width, as a pure client | v0.4.0 |
| M4 | Peers, true seed counts, recheck, turtle and global limits, ready to watch (with "may stall"), low-disk warning, in both UIs | v0.5.0 |
| M5 | Trackers, pieces, graphs, statistics, queue control, fast resume, undo, clipboard watch | v0.6.0 |
| M6 | Torznab, move, rename, tracker editing, quick-add palette, deferred add | later |

**M1a demo script** (record in the M1a receipt): (1) install the P1 build with `scripts/install.sh`
(P1-04: builds, `mv`s the binary over the running one, runs `fbittorrent setup --magnet`) while
the v0.1 TUI is open; run `fbittorrent list --json` (reads answer) and `fbittorrent stop
<hash>` (refused with "an older fbittorrent app is running; quit it with Q and retry", §3.6); quit
the old TUI with `Q`. (2) `fbittorrent status --json` → `"running": false`. (3) Click a magnet link
in the browser → no terminal; the "Added" notification with "Undo" and "Open window"; `fbittorrent
status --json` shows the download. (4) `fbittorrent tui`, see the torrent, press `q` → the one-time
notice; `fbittorrent status --json` still shows it downloading. (5) `fbittorrent events --follow
--json` in another terminal prints `torrent.completed` when it finishes; the "Download complete"
notification's "Open" starts the player. (6) Wait: `status --json` shows `daemon.idle_exit_in_s`
counting down; the daemon exits; `systemctl --user is-active fbittorrent` prints `inactive`. (7)
Click another magnet, then `fbittorrent down` → `inactive` at once, the download paused in the
ledger (the owner's stop before the popup's power button exists at M1, A1); `fbittorrent up`
resumes it.

**M1 demo script** (each step has an expected observation; record them in the M1 receipt):

1. `fbittorrent down; fbittorrent status --json` → `"running": false`; the bar icon is hidden.
2. Click a magnet link for a legal test torrent (for example a Blender open movie) in the browser →
   no terminal opens; a notification "Added" with "Undo" and "Open window" appears; within a
   second the bar icon shows the downloading glyph.
3. Open the popup → the row shows "Downloading", a moving percentage, rate and ETA; `ps` shows no
   `fbittorrent status` processes spawned by the shell; the hero meta is uppercase caption, the
   row actions appear only under the cursor.
4. Press `p` → the row shows "pausing…", then "Paused"; press `p` again → "resuming…", then
   "Downloading".
5. Press `d` → the confirm strip appears with Cancel focused; press Enter → nothing is removed.
6. Run `fbittorrent tui` in Ghostty, press `q` → the notice explains that downloads continue; the
   bar still shows activity. In another terminal `fbittorrent events --follow --json` prints the
   completion when it comes.
7. Wait for completion → a notification "Download complete" with "Open" and "Show in folder";
   "Open" starts the player.
8. Wait about five minutes → the popup's "stops in" countdown runs out, the daemon exits, the icon
   disappears; `systemctl --user is-active fbittorrent` prints `inactive`.
9. Click a second magnet; open the popup; click the power icon `󰐥` → the strip "Stop fbittorrent?
   1 download pauses until it starts again." with Cancel focused; press Enter → nothing stops;
   click Stop → the daemon drains, `fbittorrent status --json` reports `running: false`, the icon
   dims for 10 s and leaves the bar; `fbittorrent up` → the download resumes (A1: lit icon =
   running process; the power button stops it, and it is done).
10. Install a rebuilt binary with `scripts/install.sh` (copy to `fbittorrent.tmp`, then `mv` over
   the old file; a plain `cp` fails with "Text file busy" while the daemon runs) while a download
   runs → within 30 s the popup shows "Update installed; restarts when idle"; `hello` still
   reports the old `server.build`.
11. `omarchy-theme-set catppuccin-latte` → the popup re-renders in the light theme with every state
    still legible; switch back.

**M2 demo script:** open the dashboard with the keybinding; search "Sintel"; see deduplicated
results with posters and IMDb ratings; Enter stages the add; the add sheet shows files, the seed
switch and the destination; Enter downloads; the torrent appears in Torrents with a details pane;
uncheck a file in Files; press `2` with an empty query and see the poster grid; open Shows, set a
show's quality to 2160p, mark a release read, undo it; open Settings, change max downloads, see it
apply live; narrow the window below 1,400 px and see the details become a sheet; close the window
while downloading; the download continues.

**M3 demo script:** run the TUI at 80×24 over SSH; every torrent row shows a state word; Enter opens
the details modal; `[`/`]` switch its tabs; the footer never drops actions silently; `Q` asks
before stopping two active downloads; the monochrome theme still distinguishes errors (bold `!`);
`v` shows the poster card for a search result.

**M4 demo script:** start a film from Search; within the first minute the row turns "Ready to
watch" and a notification offers "Open"; press `o` and the player starts while the download
continues; throttle the connection below the bitrate (`z` with a 200 KB/s turtle limit) and the
row changes to "Playable · may stall (finishes in 18 min)" within two seconds, then back when the
limit is lifted; open Peers and see seeds and leechers with moving rates; `R` rechecks after a
deliberate corruption and the torrent heals; fill the destination with a filler file and the
"Not enough space" banner appears in the popup, the dashboard and the TUI before any torrent
errors; delete the filler and the banner clears.

---
## 10. Risks, decisions, open questions

### 10.1 Risks

| ID | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | The uncommitted tree is lost before P0-04 lands | low | critical | Back up first (P0-01); P0-04 is the first change; no destructive git commands (§9.1). |
| R18 (round 3) | The first user-visible change is at the end of P2, after 32 S-units of serial work through three L items; attention and the owner's patience run out before it | medium | high | A milestone at the end of P1 with its own demo and install (M1a, §9.3, §9.7); P1-03 and P1-19 in parallel (§9.4); the plugin lane starts against P1-06 (§9.2). |
| R19 (round 3, narrowed round 4, mostly closed round 6) | P0-02 waits on the owner for one act: creating the public repository `odcpw/asupersync` or saying "fallback". The licence questions were answered on 2026-10-18 (Q3 and Q30, "who cares, continue"); the plan still draws no legal conclusion | low | medium | P0-02's checkpoint asks once and proceeds on either answer; option B (commit C0) needs no repository and runs without asking again; the timer-wheel runner that would have broken when the tree is archived is retired (P0-02 step 5), and P0-02 depends on P0-01 so the whole-tree backup exists before step 6 removes `vendor/asupersync/`. |
| R2 | Quickshell, Omarchy or Qt changes break the plugin. Quickshell is pre-1.0 and rebuilt per Qt release; Omarchy patched 0.3.2 for Qt 6.12 and reports `4.0.0.alpha` [FO §2.1]; omaqbt broke on a Qt 6.12 colour change [OR §1.4] | medium | high | Use only documented shell components and the `shell` facade; `Commons.Color` qualified imports; keep QML thin; the TUI and CLI remain full fallbacks; the plugin checks the API version. Last resort: run the dashboard as a standalone `qs -p` instance with its own app id, which Omarchy discourages for plugins but which buys isolation [FO §7.3]. |
| R3 | A plugin bug stalls or crashes the desktop shell (bar, notifications, lock screen) [FO §2.7] | low | high | Bounded frames; no per-delta loops; no long-lived children; Store logic tested outside the shell (P2-02, P2-06); the poster grid's frame budget test (P3-17); the dashboard can be disabled by removing the `panel` kind. |
| R4 | The two UIs drift apart | medium | medium | API first (principle 6); shared fixtures; one generated keymap (P4-02); one state vocabulary (`Model.js` and `format.rs` with shared fixtures); T1 lands in both. |
| R5 | The new lifetime surprises the owner: downloads continue after closing a window, or the daemon has exited when they expected it running | medium | medium | A1 is the owner's own rule (lit icon = running process; the power button stops it); a one-time notice; the visible countdown; `Q`, `down` and the power button always work. |
| R6 | Version skew between daemon, CLI and plugin, frequent during development | high | medium | Versions in `hello`, `caps`, warnings, binary-replacement restart (P1-17), protocol checks, the compat socket period (§3.6). |
| R7 | Fast resume (P6-06) trusts data that changed or is corrupt | low | high | Trust only after a clean-shutdown marker and matching size and mtime for every file; `recheck` stays available; P1-16 covers the common case (completed torrents) without trusting anything. |
| R8 | The rqbit and Asupersync forks drift from upstream; security fixes in TLS dependencies are tracked by hand [RC §12] | medium | medium | Offer the wheel fix upstream (P0-02); a weekly scheduled `cargo audit` job (P0-06); review forks when upstream releases. |
| R9 | Download CPU stays about 3.2 times qBittorrent's [RC §1] | known | low to medium | The resource gate prevents regressions; no new optimisation work unless the owner notices heat or noise. |
| R10 | VPN binding has never been exercised against the real `proton0` interface [RC §12]; the owner's config has it off today | medium | high (privacy) | The no-fallback semantics stay; namespace fixtures in tests (`vpn.py`, including the headless park/unpark case of P1-03); before the owner turns binding back on, a manual check: disconnect Proton and confirm transfers stop and every UI shows the waiting state. |
| R11 | GitHub Actions is unavailable or slow (a release build takes about 8 minutes) | medium | low | Cache builds; tests in debug where possible; `all.sh` as the gate otherwise (§8.4). |
| R12 | Drag and drop or focus on a Quickshell `FloatingWindow` misbehaves on Hyprland [FO §2.3, §2.11] | medium | low | The focus ladder (P3-08); the clipboard is the primary add path; DnD is a manual check (P3-11). |
| R13 | The user systemd manager is missing (TTY login, container) | low | low | Exit 4 with the hint `fbittorrent serve`: the owner runs the daemon in a terminal or tmux window and every client attaches (§3.2.14). |
| R14 | Notification actions need the GUI environment inside the user manager | low | medium | Verified present on this machine (§3.2.2); without it, notifications still show, without working actions. |
| R15 (new) | Ready-to-watch fires too early for high-bitrate files, or never for files whose first piece is shared with another file | medium | low | The threshold is max(32 MiB, 3 %) and requires the last piece; `buffered_s` is shown when known so the user can judge; the per-file first piece is computed from file offsets, not torrent piece 0 (`file_info.rs:14-25` already does this per file); the fixture test covers a two-file torrent. |
| R16 (new) | The clipboard watcher reads sensitive text | low | medium | Off by default (A7); the daemon keeps only the last magnet's hash and never logs clipboard text (`daemon.py::clipboard_never_logged`). |
| R17 (new) | The visual harness depends on the live shell and the owner's session (theme switch, `grim`) | medium | low | Opt-in (`FBT_TEST_VISUAL=1`); restores the theme and the runtime-dir hook on exit (trap); a `--dry-run` checks its tools first. |

### 10.2 Decisions log

| ID | Decision | Where justified |
|---|---|---|
| DL-01 | Both surfaces over a headless daemon; no Rust GUI toolkit | §0.1 |
| DL-02 | Stop policy `idle` with 300 s, `last-ui` and `explicit` available. **Superseded by DL-70** (the owner's answer, 2026-10-18): one stop rule, no `stop_policy` setting | §0.3 A1, §3.2.10 |
| DL-03 | Keep the TUI as a pure client; no retirement review item | §0.3 A2, P7-07 |
| DL-04 | Dashboard is a `FloatingWindow` in a `panel` kind of the same plugin; no overlay in v1 | §0.3 A3 |
| DL-05 | Search, RSS, Shows and IMDb move into the daemon in P3; the CLI keeps an in-process fallback | §0.3 A4, §3.7 |
| DL-06 | NDJSON on a new `api.sock`; the 4-byte `engine.sock` stays until v0.4.0 | §3.4.1, §3.6 |
| DL-07 | A static systemd user unit with `Type=notify`; no socket activation, because a socket unit would let the bar's status connection start the engine [FO §4.2] | §3.2.2 |
| DL-08 | Strict input, tolerant output; additive minor versions; `caps` for negotiation | §3.4.3, §3.4.4 |
| DL-09 | Snapshots in chunks of at most 256 items; slow clients are closed, never fed partial data; 512-frame queues | §3.4.7 |
| DL-10 | Reads never start the daemon, writes do; stop and keep-files removal work offline on the ledger | §3.2.3 |
| DL-11 | The popup offers removal with files kept only | §4.5 |
| DL-12 | Middle click on the bar icon opens the dashboard (was: the TUI); `middleClick = "tui"` restores it | §4.5 |
| DL-13 | The bar widget stays hidden while fbittorrent is stopped (owner rule, confirmed 2026-10-18 with Q1: a lit icon means a running process); `showWhenStopped` exists, off | §4.5 |
| DL-14 | Legacy qBittorrent mode is removed in v0.4.0; `seq`, `first-last` and `reconcile` remain as stubs, so the CLI contract stays 1.x | P4-09 |
| DL-15 | Asupersync through a pinned git fork; rqbit and nova3 stay in-tree | §8.2 |
| DL-16 | Validation receipts leave git except READMEs and small linked files | §8.3 |
| DL-17 | The Omarchy marketplace is not a goal | §7.1 |
| DL-18 | No further CPU optimisation work; a resource regression gate only | §1.7 |
| DL-19 | `seeds` means connected seeds and is labelled so; no tracker scrape | §6.3, §2.6 |
| DL-20 | Staged add in the dashboard; quick add in the popup, CLI and magnet handler; `handler = "confirm"` stages from the handler too | §4.8, A5 |
| DL-21 | Sorting in QML for now, with a measured threshold for moving it to the server | §4.4 |
| DL-22 | `status` while stopped answers `ok: true, running: false` (contract 1.2.0) | §3.2.13 |
| DL-23 (new) | Superseded by DL-31 (round 3): one ladder for both surfaces. | §4.2.1 |
| DL-24 (new) | Row actions in the popup appear on hover or cursor only | §4.3.1 |
| DL-25 (new) | Every confirm dialog sets `selectedIndex: 0` because the shell's default is the destructive button | §4.9 |
| DL-26 (new) | Ready-to-watch threshold: contiguous prefix ≥ max(32 MiB, 3 % of the file) plus the file's last piece; one event per torrent per daemon life | P5-05 |
| DL-27 (new) | Clipboard watching is off by default and never keeps the daemon alive | A7, §3.2.10 |
| DL-28 (new) | Ref ring keyed by `(client.name, ref)`; refs do not survive a daemon restart; `remove` is refused after a restart | §3.4.8, §3.2.11 |
| DL-29 (new) | ftui stays at 0.6 for M3; its widget set suffices | §1.7, §5.8 |
| DL-30 (round 2) | Dashboard geometry is computed, not borrowed: panes 200/360, fixed columns 748 px, NAME floor 210 px, tiers ≥ 1600 / 1400 / 1100 / 900 with monotonic column visibility, default window 1600×900 | §4.2.2, §4.3.2, §4.3.3 |
| DL-31 (round 2, corrected round 3) | Dim text is computed by one ladder (1.4 → mix 0.75 → mix 0.85 → foreground) in QML and TUI, defined once in §4.2.1, from one theme fixture (`contracts/ui/themes/`); `muted` and `dark_foreground` are never text colours; accent falls back to bold foreground below 4.5:1 (§5.5.1); the TUI's text gate is 4.5:1, not 7:1 | §4.2.1, §5.5.1, P2-08, P4-05 |
| DL-32 (round 2) | One state table (`contracts/ui/states.json`) feeds popup, dashboard, TUI and CLI; `◌` is metadata, `…` stays queued/waiting as the Shows screen already uses it | §4.3.4, §5.5.3 |
| DL-33 (round 2, extended round 3) | Space pauses and resumes in every surface (`returnRequested` tells Enter from Space); `q` closes the surface it is pressed in and `Q` quits, everywhere; the popup's Enter (opens the file) and `w` (opens the window) are the two key exceptions, listed in Appendix B | §4.12, Appendix B |
| DL-34 (round 2) | Keys: `U` undo-remove, `=` show quality, `P` providers on Search, `s` sort on every list, `y` copy magnet; the dashboard closes on `q`, Esc at the top level never closes it; double-click is Enter; no context menus | Appendix B, §4.3.2 |
| DL-35 (round 2) | The popup's quick add (`a`, right click) shows the P6-13 offer strip with the name before adding; the handler and the CLI stay immediate | §4.8 |
| DL-36 (round 2) | A glance (open popup) pauses the idle countdown without counting as interactive; the icon never hides under an open popup; a 10 s "stopped recently" state makes the exit visible | §3.2.10, §4.5 |
| DL-37 (round 2) | One bar mark: `󰇚` for idle, stopped and downloading, state by weight; `󰕒` only while seeding with nothing downloading | §4.5, §4.2.4 |
| DL-38 (round 2) | Popup width `Style.space(380)`, the first-party width; key hints are one caption line | §4.2.2, §4.2.3 |
| DL-39 (round 2) | `playable.stall_free` next to `ready`: the notification fires at the DL-26 threshold, the row says "may stall" while the rate cannot keep ahead of playback; `wait --until stall-free` | §3.5, P5-05 |
| DL-40 (round 2) | Low-disk banner at `free < needed + 512 MiB`, `reason: disk_full` on `ENOSPC`, the add sheet refuses oversize adds; T1 | P5-07, §6.2 row 37 |
| DL-41 (round 2) | `events` is a topic with a `seq` ring (1,000 entries or 10 min); `events --since-seq` replays and reports `resync` when the ring is too short | A.7, A.8, P1-18 |
| DL-42 (round 2) | `Pieces.have` is a base64 bitfield (bounded by construction), not run-length pairs | A.9, P6-02 |
| DL-43 (round 2) | `busy` maps to exit 4 (retryable never maps to 1); `status --quiet` exits 4 when stopped; the `status` stopped-shape fix is a declared break in `capabilities.changes[]` | A.3, Appendix F, P1-11 |
| DL-44 (round 2) | `autostart = false` (the existing key) is the persistent form of `--no-start`; `up`, `tui`, `dashboard`, `serve` still start | §3.2.3 |
| DL-45 (round 2) | Unit: `KillMode=mixed`, `StartLimitIntervalSec=60`, `StartLimitBurst=20`, because an on-demand unit starts often by design | §3.2.5, P1-04 |
| DL-46 (round 2, corrected round 3) | Commit C3 carries sources, manifest, lock and the `[patch]` entry together (no unbuildable intermediate); option B for Asupersync is the full vendored tree (a packaged copy is not smaller: `src/` is 56 MB) | §8.1, §8.2 |
| DL-47 (round 2) | P1-02 is the isomorphic file split only; P1-19 adds the typed operations and the normalized model, so the isomorphism test means something | §9.3 |
| DL-48 (round 2) | Draining quiesces first (stop accepting, answer in-flight requests, then `bye`); clients resend idempotent ops with the same `ref` after a mid-request loss and exit 8 for `remove`/`recheck`/`move` | §3.2.4, §3.2.7 |
| DL-49 (round 2) | The slot is `Running` as soon as the session exists; restores run in background tasks and show as `checking`/`metadata` | §3.2.4, P0-09 |
| DL-50 (round 2) | Structural frames are event-driven through a fork accessor over `state_change_notify`; only rates and progress ride the tick | §3.4.7, P1-08 |
| DL-51 (round 2) | Search results carry `added` (the ledger entry with the same btih) and show `+ In your list`; Enter selects it instead of adding | A.9, P3-12 |
| DL-52 (round 2) | The CLI notifies "Added" only on a TTY or with `--notify`; completion and ready notifications open on a body click (`default`) | §3.8 |
| DL-53 (round 3) | One dim ladder for QML and TUI, defined once with its formulas (Qt HSV darken 1.4 → mix 0.75 → mix 0.85 → foreground), `±1` per channel against `expected.json`; `muted` and `dark_foreground` are never text colours (corrects DL-31; supersedes DL-23) | §4.2.1, §5.5.1 |
| DL-54 (round 3) | `q` closes the surface it is pressed in and `Q` quits, everywhere; Esc never closes the dashboard; the popup's Enter and `w` are the only two key exceptions, listed once in Appendix B (extends DL-33, DL-34) | §4.5, §4.12, Appendix B |
| DL-55 (round 3) | The idempotent operations are listed once (A.1); requests carry `instance` so a resend after a restart can be told from a fresh request; the ref ring's eviction, reuse and replay rules are P1-09's | A.1, §3.2.4, §3.2.11, P1-09 |
| DL-56 (round 3) | The harness is a skeleton before the daemon exists (P1-07 depends on P0-15; P1-03 on P1-07); `ensure_daemon()` probes `engine.sock` until P1-11; P1-03 depends on P1-02 and not on P1-19; there is no Rust twin of the test harness | §9.3, §9.1 |
| DL-57 (round 3) | M1a, the end of P1, is a milestone: installed, not tagged; its receipt records the one-time compat check of §3.6 | §0.5, §9.3, §9.7 |
| DL-58 (round 3) | Option B for Asupersync is the full vendored tree (a `cargo package`-shaped copy is not smaller: `src/` is 56 MB); Q3 was the only owner action before P0-04 (corrects DL-46); answered 2026-10-18, the fork, with option B as the runnable fallback (DL-70) | §8.2, P0-02, Q3 |
| DL-59 (round 3) | Cuts of machinery: legacy-binary compatibility tests (the TUI of P1-15 is the compat adapter's only consumer); the 1 px automated comparison against the Wi-Fi panel (the owner judges the pair image); `scripts/free-chord.sh`; `scripts/check-fixtures.sh`; `proto_minor` and the per-minor changelog test (`caps` is the negotiation); `--self-test` modes; CI jobs for `dagcheck.py`, `mockwidth.py` and `check-doc-links.sh` (run by hand); `Summary.free_space` and `Summary.daemon.clients`; counters in `stats.json`; captures under `docs/validation/`; P7-08 and §7.4's marketplace build design | §9.3, §8.4, §4.16, A.9, §7.4 |
| DL-60 (round 3) | Plugin tests live in `plugin/tests/`, outside the symlinked plugin folder; captures go to `target/test-logs/visual/` | §4.1, §4.16 |
| DL-61 (round 3) | `Settings` keys are the `config.toml` keys; the four limit keys are `*_limit_bps` plus `alt_limits_on`, so the turtle state survives a restart | A.9, §6.3 |
| DL-62 (round 3) | Client-side request timeouts are 20 s (100 s for `torrents.add`), longer than the server's 15 s and 95 s; `log` and `events` exit 4 when the daemon is down; `interval_ms` applies to every topic of a `subscribe` call and A.7's per-topic defaults apply when it is omitted | §4.4, §3.2.3, A.6 |
| DL-63 (round 3) | The three UI contract files have shapes: `contracts/ui/states.json` (§4.3.4), `contracts/ui/themes/expected.json` (§4.2.1), `keymap.json` (Appendix B, action-keyed with per-surface `keys`); `Receipt` is a section of `docs/validation/<date>/README.md` (Appendix D) | §4.3.4, §4.2.1, Appendix B, Appendix D |
| DL-64 (round 3) | The dashboard's Shows view stages on Enter like Search and RSS (the TUI downloads directly); bulk remove is keep-files only; undo is per batch; `display_name` is an override that an empty name reverts | P3-13, P6-08, P7-03 |
| DL-65 (round 3) | Every §9.3 item is self-contained to the standard of the §9.3 preamble; the P6 and P7 tables are bullet items; §9.4 is generated from §9.3 by `scripts/dagcheck.py`'s sibling and never hand-edited | §9.3, §9.4 |
| DL-66 (round 4) | The M1a chain is closed: `scripts/install.sh` is P1-04's (P2-07 extends it) and runs `setup --magnet`; the v0.1 popup's `online` test gains `status.running !== false` (P0-07) so it hides under the 1.2.0 stopped shape; the running `status --json` shape keeps every 1.x field (P1-11); the daemon posts the actioned "Added" toast only for `origin: handler` and the CLI keeps its plain one (P1-13); an operation during `Starting` waits up to 60 s for the session (the §9.3 preamble agrees with §3.2.4); `start` on a completed stop-on-complete record is `invalid_state`; `set_files` with a non-empty `include` restores a dormant torrent (P1-16) | §0.5, §3.2.13, §8.6, §9.3, §9.7, P0-07, P1-04, P1-11, P1-13, P1-16 |
| DL-67 (round 4) | Protocol holes filled: a request-less error is the event `error` followed by `bye` (A.1, A.8); `subscribe` takes `since_seq` and `snapshot.begin` carries `oldest_seq` on resync; `subs` is 16; the coalesced field set is closed (§3.4.7); the 1 MiB line cap binds every pushed frame and every result the plugin asks for, while `torrents.list` and `torrents.files` are uncapped and banned in `plugin/` by lint (P2-05); the bus selects over `state_change_notify` and `finished_notify` and waits on `Ledger.generation` plus a `Condvar` (P1-19), never on the async mutex; client kinds are `cli`, `agent` (the CLI under `--json`), `tui`, `bar`, `test`; the dashboard's bulk `X` is a window `Shortcut`; `reconnecting` keeps the bar icon through a crash-restart | §3.4.1, §3.4.3, §3.4.7, §4.4, §4.5, A.1, A.2, A.4, A.6, A.7, A.8, P1-06, P1-08, P1-19, Appendix B |
| DL-68 (round 4) | Cuts of machinery: `serve --test-proto` (the mismatch banner is tested from a `proto: 2` fixture frame), `serve --detach` with the double fork, `daemon.log` rotation and `last-daemon.json` (no user manager → exit 4 with the hint `fbittorrent serve`; `instance` is 32 hex characters from `/dev/urandom`), the timer-wheel runner retargeting (the fork's `cargo test` is the test of record, and the two runner files leave C5), the `ping` operation, the per-torrent speed rings. The `last-ui` and `explicit` policies were cut when the owner answered Q1 and Q31 (DL-70) | §3.2.1, §3.2.4, §3.2.7, §3.2.10, §3.2.14, §8.1, A.2, A.4, P0-02, P1-03, P1-04, P1-06, P1-12, P6-03, Appendix F |
| DL-69 (round 4) | The popup's key-hint row is hidden at rest and shown from the first key press (assumed answer to Q32); contract-1.x numeric fields keep qBittorrent's sentinels (`eta` 8640000, `num_seeds` 0, `progress_known: false`) with the truth in `eta_s`, `seeds` and `native_state`, declared in `capabilities.changes[]`; sizes are binary units everywhere, "GiB" in sentences and "1.4G" in table cells, never "GB"; `LICENSE` (MIT) and `NOTICE` land in P0-03; the width tiers are `Style.space` units chosen from `width / Style.effectiveSpacingScale`; the TUI details modal opens on Overview at every width; the TUI tab bar's daemon-state text replaces the rate text only while nothing transfers; Torznab keys live only in `api_key_file`, with a 4 MiB and 30 s cap | §4.2.3, §4.3.3, §5.6, §5.7, §7.3, P0-03, P1-11, P2-03, P4-07 |
| DL-70 (the owner, 2026-10-18) | Q1 and Q31, "I don't want running processes. When we close the toolbar widget, it stops, and is done.": one stop rule. The bar icon is visible exactly while the daemon runs; the icon's power button, `fbittorrent down` and `Q` in the TUI stop the daemon with everything in it (the popup and the TUI confirm when transfers are active); closing the TUI or the dashboard never stops a transfer, dismissing the popup never stops anything, and the bar never keeps the daemon alive; the idle net `idle_exit_secs = 300` stays on by default and `0` disables it; the `stop_policy` enum (`idle`, `last-ui`, `explicit`) and its daemon and UI branches are deleted. Q2 with it: hidden when stopped. Q3 and Q30, "who cares, continue": the public fork `odcpw/asupersync` with the full tree as the runnable fallback, one owner checkpoint before any push; the owner accepts the rider's "available to" reading for the project, and the plan still draws no legal conclusion | §0.3 A1, §0.4, §2.2, §3.2.10, §4.5, §8.2, P0-02, P1-12, P1-15, P2-03, A.2, A.9, Appendix B, Appendix F, §10.3 |

### 10.3 Open questions for the owner

Each has a default the plan already uses, so none blocks work. Only questions whose answer is
genuinely the owner's remain; round 1 removed Q7 (middle click) and Q8 (marketplace) because the
plan already decided them with a setting and a non-goal, and added three. Six are answered (Q1,
Q2, Q3, Q4, Q30, Q31; 2026-10-18): their rows keep the question and carry the owner's words.

| # | Question | Default |
|---|---|---|
| Q1 (**answered 2026-10-18**) | Is "keep running while there is work, then stop after 5 idle minutes" right? Or do you prefer "stop when the last window closes" (closest to today) or "run until I quit"? If "idle" is right, Q31 asks whether the two alternatives may go. | **Answer:** "I don't want running processes. When we close the toolbar widget, it stops, and is done." Encoded as A1 and DL-70: the bar icon is lit exactly while the daemon runs; its power button, `down` and `Q` stop it with everything in it; closing windows never does; the idle net stays at 300 s (`0` disables it); no `stop_policy` setting. |
| Q2 (**answered 2026-10-18**, with Q1) | Should the bar icon stay visible (dimmed) while fbittorrent is stopped, so one click starts it? Round 2 added the 10 s "stopped recently" state and the glance pause (§4.5, §3.2.10), which remove the vanish-under-the-cursor case either way. | **Answer:** hidden when stopped, so that a lit icon always means a running process (DL-70); the 10 s stopped-recently dim stays; `showWhenStopped` stays off. |
| Q3 (**answered 2026-10-18**; it decided what goes into history) | May a public fork `odcpw/asupersync` hold the patch (about 1,300 lines over 10 files, P0-02; you create the repository, since publishing is your act; builds need network once), or should the full 56 MB `vendor/asupersync/` tree be committed (§8.2 option B, commit C0; no second repository, no network, 56 MB in every clone)? Three facts bear on it: `vendor/asupersync/LICENSE` is "MIT License (with OpenAI/Anthropic Rider)", which every distribution must carry unmodified and which restricts the named parties (the plan draws no legal conclusion; the fork is a distribution and so is option B); the rider also forbids making the Software or a derivative "available to" those parties and counts "analyzing" and "testing" as use, and this plan is executed by agents of both parties that read `vendor/asupersync/` and the binary that links it, so the question is not only where the patch lives but whether you accept that reading of the rider for the whole project; and `odcpw/fbittorrent` itself is private, so a private fork would need git authentication for cargo and CI (`net.git-fetch-with-cli`), which the plan does not cover. | **Answer:** "who cares, continue": the public fork; the full tree stays the runnable fallback if the repository is not created (P0-02's checkpoint asks once; DL-70). |
| Q4 (**answered 2026-10-18**) | Where should the validation archive (about 279 MB, outside git) be backed up? | **Answer:** "no archive; data deleted, README summaries kept in git" (§8.3, P0-01). |
| Q5 | Should a followed show ever be able to download new episodes automatically, as an opt-in per show? | no; checking never downloads |
| Q6 | Which free key chord should toggle the dashboard? (SUPER+CTRL+T is Activity.) | chosen in P2-07 from the free chords |
| Q9 (new) | Should a `magnet:` click add immediately (notification with Undo) or open the add sheet first? | add immediately (`handler = "add"`) |
| Q10 (new) | Is "ready to watch" at 32 MiB / 3 % the right moment for your connection and player for the **notification**, with the row saying "may stall" while the current rate is below the bitrate (§3.5, P5-05), or should the notification itself wait until playback cannot stall? | notify at the threshold; the row says "may stall" |
| Q11 (new) | Do you want clipboard watching turned on after M5, knowing the daemon then reads every clipboard change while it runs? | off |
| Q12 (round 2) | `status --json` while stopped: accept the declared 1.2.0 break (`ok: true, running: false`, Appendix F) now, or keep `ok: false` with the added `running` field until a 2.0.0? | the break, declared in `capabilities.changes[]` |
| Q13 (round 2) | `autostart = false` is in your config today. Keep it as the persistent form of `--no-start` (§3.2.3), or retire it in favour of `--no-start` only? | keep |
| Q14 (round 2) | Keys: `U` for undo-remove and `=` for show quality (so `u` stays seed and `Q` stays quit), `P` for Search's providers sheet, `y` to copy a magnet, `q` to close the dashboard (Esc never closes it)? | as listed |
| Q15 (round 2) | Player: `o` through `xdg-open` (whatever handles `.mkv`), or an explicit `open_command` such as `mpv --keep-open=yes`, which keeps playing a growing file where some players stop at the first EOF? | `xdg-open`; `open_command` in Settings |
| Q16 (round 2) | Peers tab tier: the true seed count (P5-01) fixes D6 on its own; is the Peers tab (P5-02) T1 for you, or may it be T2 (dashboard first) so M4 ships earlier? | T1 as planned |
| Q17 (round 2) | Will the dashboard mostly be tiled at about 960 px (then the chip-row tier is the one to perfect) or floated wide (the 1600 px tier)? | both are designed; tiled is the daily case |
| Q18 (round 2) | Space pauses in the dashboard and the popup as in the TUI (§4.12), or Space activates as in the Wi-Fi panel? | pauses |
| Q19 (round 2) | Popup quick add: `a` then Enter with the name shown (§4.8), or one key, immediate, as today? | strip, then Enter |
| Q20 (round 2) | "In your list" on search results (A.9 `added`): wanted, or is the duplicate answer on add enough? | wanted |
| Q21 (round 3) | Popup `q`: round 3 makes `q` close the popup and `Q` (or the power icon) quit, so `q` means "close this" in every surface (Appendix B). Today's popup stops the daemon on `q`. Acceptable? | `q` closes, `Q` quits |
| Q22 (round 3) | A demo checkpoint after P1 (M1a): worth installing a build whose popup is still the old polling one, in exchange for living with the new lifetime several weeks before the popup rewrite? | yes, install M1a |
| Q23 (round 3) | Look-and-feel gate: a side-by-side capture of our popup and the Wi-Fi panel that you judge (§4.16 `--compare`), instead of an automated 1 px comparison against the Wi-Fi panel's screenshot? | the judged pair |
| Q24 (round 3) | Dim colour in the TUI: with one ladder (§4.2.1) the TUI's secondary text on your theme becomes `#b6b6b6` (10.4:1), identical to the popup, rather than `muted` `#7a7a7a` (4.9:1). Brighter and identical, or a TUI-only rung at `muted` when it passes 4.5:1 (then the two surfaces differ on your theme, by design)? | identical (`#b6b6b6`) |
| Q25 (round 3) | Accent on the monochrome theme: under the §5.5.1 rule your `#8d8d8d` accent is used as is for keys and the ready badge (6.3:1 on black, clearly darker than white text). Grey accent, or bold white? | grey accent, as the rule says |
| Q26 (round 3) | Shows: Enter in the dashboard stages a release in the add sheet like Search and RSS (P3-13), the TUI downloads directly (W2). Confirm, or make Shows the one direct-download view in the dashboard? | stage |
| Q27 (round 3) | Rename revert: `Record.display_name` keeps the torrent's own name recoverable (an empty name reverts). If you never want the original back, `Record.name` alone is simpler. | keep the override |
| Q28 (round 3) | Bulk remove is keep-files only (`X`); deleting files stays per torrent. Do you want a bulk delete-files (its own confirm text, no undo)? | no |
| Q29 (round 3) | Undo scope: `undo` with no hash restores the last removal, one or many (per batch), as P6-08 specifies; or per hash only? | per batch |
| Q30 (round 4, **answered 2026-10-18**) | The Asupersync rider (`vendor/asupersync/LICENSE`) also forbids making the Software or a derivative "available to" OpenAI and Anthropic and their agents, and names "analyzing" and "testing" as use. Every phase of this plan has Claude Code or Codex read `vendor/asupersync/` (or the fork) and build the binary that links it. Do you accept that reading of the rider for the whole project, independent of where the patch is hosted (Q3)? The plan draws no legal conclusion. | **Answer:** "who cares, continue": accepted; the plan proceeds as written and still draws no legal conclusion (DL-70, R19). |
| Q31 (round 4, **answered 2026-10-18**, with Q1) | If Q1 stays "idle": may the `last-ui` and `explicit` stop policies go, with `idle_exit_secs = 0` meaning "never exit on its own"? `last-ui` is the behaviour you are leaving (D9); `explicit` is a number. Removing them deletes an enum, two daemon cases and two UI branches (P1-12, P1-15), nothing you would see. | **Answer:** yes, and further: `explicit` is superseded by the power button plus the idle net, so there is no `stop_policy` setting at all (DL-70, P1-12). |
| Q32 (round 4) | The popup's key-hint row ("a add · / search · w window…"): hidden until the first key press of an opening, so the popup at rest has the Wi-Fi panel's structure (no first-party popup has a hint row), or always visible as round 3 specified, so a keyboard user sees the keys at once? | hidden at rest (§4.2.3, P2-03) |
| Q33 (round 4) | The "Added" toast: the daemon posts it with "Undo" and "Open window" only for browser clicks (`origin: handler`); `fbittorrent add` in a terminal keeps a plain toast with no actions, because an actioned toast blocks the command for the toast's lifetime (`notify-send -A` implies `--wait`). Fine, or should a TTY `add` also get the actioned toast? | plain toast on a TTY (P1-13) |

### 10.4 Unverified claims, and the item that verifies each

| Claim | Where | Verified by |
|---|---|---|
| An isolated `qs -p` sees `qs.Ui`/`qs.Commons` through `Commons`/`Ui` symlinks in its config root (a `QML_IMPORT_PATH` cannot work) | §4.16 | P2-06 (`probe-ui.qml`; the outcome goes to `plugin/tests/README.md`) |
| Omarchy's plugin watcher (`inotifywait -m -r` over `~/.config/omarchy/plugins`, `services/PluginRegistry.qml:716-727`) reloads the shell when a file changes inside the symlinked `oliver.torrents` folder; `inotifywait -r` does not follow symlinked directories, so §4.15's hot-reload claim may hold only for `setup --plugin=copy` | §4.15 | P2-01 (`qs-dashboard.sh::reload_reconnects` logs "symlink reload: yes/no"; if no, §4.15 and the README say `--plugin=copy`) |
| `hyprctl output create headless` works on this Hyprland | P2-01 | P2-01 (skips with a logged reason) |
| Drag and drop onto a `FloatingWindow` works on Hyprland | §4.8 | P3-11 (manual check logged) |
| GitHub Actions minutes are available | §8.4 | P0-06 |
| nova3 file licences permit MIT embedding | §8.2 | P0-03 |
| Explicit caps on RSS, IMDb and poster reads | §7.2 | P3-02, P3-04 |
| The fork honours the private flag for DHT and PeX | §6.2 row 16 | P6-12 |
| A container duration can be read cheaply from `mkv`/`mp4` headers for `buffered_s` | P5-05 | P5-05 (omit the field if not) |
| `wl-paste --watch` output can be bounded without losing events | P6-13 | P6-13 |
| Whether the ftui runtime clears the pending signal on a dead pty (which of P0-08's two fixes is needed) | P0-08 | P0-08 instrumentation |

Round 0's "unverified" list also contained: the `notify-send -A` action flag (now verified,
§3.8), `systemd-inhibit` presence (verified), the user manager's `WAYLAND_DISPLAY` (verified),
summoning a panel referenced only from the bar layout (verified, §4.1), and ftui's signal
mechanism (verified, D2).

### 10.5 How this plan gets better before implementation

Four review rounds are done (§0.7) and round 4 judged the plan at steady state: its findings were
gaps, not redesigns. Each round followed the planning workflow: a review by a strong reasoning
model (the "Plan Review" prompt of the `planning-workflow` skill), the changes integrated in place,
and the validation loop run between rounds: pick the most obscure item and check it is
implementable alone (round 1's self-check used P5-06 and P2-08; round 2's used P5-07 and P1-19;
round 3's validator failed 0 of 8 and the integration then rewrote every item and re-sampled eight
in two batches; round 4 sampled four more, `review-log.md`); redraw the DAG of §9.4; sample five
decisions for their "why"; diff against the previous round and stop when changes are cosmetic.
`review-log.md` records each round. On 2026-10-18 the owner answered the questions that gated
work (Q1, Q2, Q3, Q30, Q31; DL-70), so nothing blocks the next step: convert §9 into beads with
`br`, keeping every dependency edge of §9.4, so agents can take ready work with `br ready --json`.

---
## Appendix A. API v1 message catalogue

Types: `u64`, `i64`, `f64` (JSON numbers), `bool`, `str`, `hash` (40 lowercase hex), `ts` (Unix
seconds, `u64`), `bps` (bytes per second, `u64`), `bytes` (`u64`), `[T]`, `T?` (optional; absent
when unknown or not applicable), `enum(a|b)` closed, `open(a|b|…)` open (clients map unknown
values to "other"). Every `args` object rejects unknown keys (`invalid_args`, `details.key`).

After P1-05, `docs/IPC.md` is the living contract and the only document tests compare against;
this appendix is its initial content and is not maintained afterwards.

### A.1 Envelope rules

- Requests: `{"id": u64 ≥ 1, "op": str, "args": object?, "ref": str? (≤ 64 ASCII), "instance":
  str?}`. `args` may be omitted when an operation takes none. `instance` is the daemon instance
  the client last saw in `hello`; clients send it on every resend after a reconnect (§3.2.4 step
  6) so the daemon can tell a request that predates its own start (P1-09).
- Responses: `{"id", "ok": true, "result": object}` or `{"id", "ok": false, "error": Error}`.
- Events: `{"ev": str, …}`; subscription events carry `sub`, `topic` and `rev`.
- An error with no request to answer (a line over the cap or not an object, `hello_required`,
  `busy` for the 33rd connection, `proto_unsupported`) is sent as the event
  `{"ev": "error", "error": Error}` and is always followed by `bye` and the close (A.8);
  `invalid_args` for a bad `id` (0, negative, non-integer, reused) answers with `"id": null`
  and keeps the connection.
- Times are Unix seconds (`*_at`); rates are bytes per second (`*_bps`); sizes are bytes; progress is
  a number from 0 to 1; durations are seconds (`*_s`).
- Full objects (snapshot items, `added`, results) never contain `null`: an unknown field is omitted.
  In a `delta`, a field set to `null` means "this field is now absent"; the client deletes it.
- Hashes are 40 lowercase hex characters (BitTorrent v1 info hash).
- Lines: client → server ≤ 65,536 bytes; server → client ≤ 1,048,576 bytes.
- Idempotent operations (safe to resend with the same `ref` after a mid-request loss, §3.2.4, or
  against a new instance after a crash, §3.2.11): `torrents.start`, `torrents.stop`,
  `torrents.set_seed`, `torrents.set_files`, `torrents.set_upload_limit`, `torrents.add`,
  `torrents.commit`, `torrents.discard`, `torrents.force_start`, `app.settings.set`,
  `transfer.set_limits`, `transfer.set_alt`, `search.set_provider`, `rss.set_read`,
  `rss.mark_feed_read`, `shows.set_quality`, `shows.set_read`, `shows.mark_show_read`,
  `torrents.reannounce`, `torrents.set_download_limit`, `torrents.add_tracker`,
  `torrents.remove_tracker`. Every other
  mutating operation (`torrents.remove`, `torrents.recheck`, `torrents.move`, `torrents.rename`,
  `torrents.queue_move`, `rss.add`, `rss.remove`, `shows.follow`, `shows.unfollow`,
  `shows.undo_read`, `search.install`, the `*.grab` family, `app.shutdown`, `app.restart`) is not,
  and a client never resends it on its own. `api::ops::TABLE` carries this flag and `api --list`
  prints it (P1-18); this list is the source.

### A.2 `hello`

| Direction | Fields |
|---|---|
| args | `proto: u64` (required; 1), `client: {name: str (≤ 64), version: str, kind: enum(cli\|agent\|tui\|bar\|test)}` (required; §3.4.3), `interactive: bool` (default false) |
| result | `proto: u64`, `server: {version: str, build: str, pid: u64, launcher: enum(systemd\|foreground), log: str}` (`launcher` is `systemd` when `NOTIFY_SOCKET` was set at start; `log` is `journalctl --user -u fbittorrent` under systemd and `the terminal running fbittorrent serve` otherwise, ready to print as a hint), `engine: str`, `instance: str` (32 lowercase hex characters from 16 bytes of `/dev/urandom`, read with `std::fs`; no uuid crate), `engine_epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `caps: [str]` (§3.4.3), `limits: {line_in: u64, line_out: u64, inflight: u64, subs: u64, hashes: u64}`, `policy: {idle_exit_secs: u64}` (0 means the idle net is off, §3.2.10) |

### A.3 Error codes

`Error = {code: str, message: str (≤ 200), hint: str?, retryable: bool, details: object?}`.

| Code | Meaning | Retryable | `details` | CLI exit |
|---|---|---|---|---|
| `invalid_message` | not JSON, not an object, or over the line cap | no | `{bytes?}` | 2 |
| `hello_required` | the first frame was not `hello`, or it came too late | no | | 2 |
| `proto_unsupported` | unsupported major version; the hint says what to update | no | `{server_proto}` | 2 |
| `unknown_op` | unknown operation; the hint lists up to three near names | no | `{nearest: [str]}` | 2 |
| `invalid_args` | missing, unknown or mistyped argument | no | `{key, expected?}` | 2 |
| `not_found` | unknown hash, feed, show, search session, file index or ref | no | `{hash?\|feed?\|imdb?\|search_id?\|index?}` | 6 |
| `ambiguous` | a selector matched several objects (raised by the CLI before calling) | no | `{matches: [str]}` | 7 |
| `safety` | destructive intent not confirmed (CLI `--yes` path) | no | | 3 |
| `invalid_state` | the operation does not apply now (resume on `done`, commit on a non-staged torrent, remove after a restart with a stale ref) | no | `{state?, reason?}` | 1 |
| `duplicate` | the torrent already exists (only as a `result` field for `add`; never an error) | n/a | | 0 |
| `unsupported` | refused by design (high or max file priority, sequential toggles) | no | | 2 |
| `engine_unavailable` | the engine is starting, parked or stopping | while starting: yes | `{state, message}` | 4 |
| `already_running` | a second `serve` while one holds the lock | no | `{pid?}` | 1 |
| `upstream` | a provider, feed, tracker or website failed | yes | `{source}` | 5 |
| `timeout` | the operation did not finish in time; its outcome is unknown | yes | `{after_s}` | 8 |
| `busy` | a limit was reached (connections, sessions, dispatch queue) | yes | `{limit}` | 4 (the hint says "retry in a moment"; 1 would contradict `retryable`) |
| `internal` | a bug; the hint points at `fbittorrent log` | no | | 1 |

Every CLI exit code 1..8 is reachable from this table (Appendix F lists the verbs that raise each).

### A.4 Operations: `app` and `transfer`

| Op | Args | Result | Mutating | Since |
|---|---|---|---|---|

| `app.info` | none | hello's `server`, `engine`, `instance`, `engine_epoch`, `state` plus `uptime_s: u64`, `upgrade_pending: bool`, `clients: {cli: u64, agent: u64, tui: u64, bar: u64, test: u64}` (connections since start), `subscriptions: {total: u64, by_topic: {str: u64}}` (live subscriptions, P1-08) | no | P1-06, P1-08 |
| `app.client` | `interactive: bool?`, `glance: bool?` (either or both; a connection drop resets both; §3.2.10) | `{}` | no | P1-06 |
| `app.shutdown` | `reason: str?` | `{state: "pending"}`, then `bye` | yes | P1-06 |
| `app.restart` | `scope: enum(engine\|daemon)` | `{state: "pending"}` | yes | P1-06, P1-17 |
| `app.settings.get` | none | `Settings` | no | P3-05 |
| `app.settings.set` | any subset of `Settings` fields | `{applied: [str], restart: bool}` | yes | P3-05 |
| `app.network` | none | `{state: NetworkState, interfaces: [str]}` | no | P1-06 |
| `app.log` | `since_seq: u64?`, `limit: u64?` (≤ 1,000, default 200), `level: enum(error\|warn\|info\|debug)?`, `source: str?`, `hash: hash?` | `{entries: [LogEntry], next_seq: u64}` | no | P1-14 |
| `app.stats` | `history: bool?` | `Stats` | no | P1-14, P6-04 |
| `app.free_space` | `path: str` | `{free_bytes: bytes?, writable: bool}` | no | P6-11 |
| `app.clipboard.dismiss` | `hash: hash` | `{}` | no | P6-13 |
| `transfer.info` | none | `{dl_bps: bps?, ul_bps: bps?, session_downloaded: bytes, session_uploaded: bytes}` | no | P1-06 |
| `transfer.set_limits` | `download_bps: bps?`, `upload_bps: bps?`, `alt_download_bps: bps?`, `alt_upload_bps: bps?` (each a number, or `null` for unlimited; omitted keys are unchanged) | `{limits: Limits}` | yes | P5-04 |
| `transfer.set_alt` | `on: bool` | `{alt: bool}` | yes | P5-04 |

### A.5 Operations: `torrents`

| Op | Args | Result | Mutating | Since |
|---|---|---|---|---|
| `torrents.list` | `states: [state]?`, `origin: [str]?` | `{torrents: [Torrent]}` | no | P1-06 |
| `torrents.get` | `hash: hash` | `{torrent: TorrentDetail}` | no | P1-06 |
| `torrents.add` | `source: {magnet: str} \| {url: str} \| {path: str} \| {restore: hash}` (exactly one), `stage: bool?`, `stopped: bool?`, `seed: bool?`, `save_path: str?`, `files: [u64]?`, `category: str?`, `origin: open(manual\|search\|rss\|shows\|clipboard\|handler\|agent)?` | `{state: enum(pending\|done), hash: hash?, name: str?, duplicate: bool?, deferred: bool?}` | yes | P1-06, P1-09, P3-06, P7-09 |
| `torrents.commit` | `hash: hash`, `files: [u64]?`, `seed: bool?`, `save_path: str?` | `{state: "done"}` | yes | P3-06 |
| `torrents.discard` | `hash: hash` (never deletes files) | `{state: "done"}` | yes | P3-06 |
| `torrents.start` | `hashes: [hash]` (1..256), `force: bool?` | `{state: "done", started: [hash], skipped: [{hash, reason: open(complete_no_seed\|already_running\|staged)}]}` | yes | P1-06, P6-05 |
| `torrents.stop` | `hashes: [hash]` | `{state: "done", stopped: [hash]}` | yes | P1-06 |
| `torrents.remove` | `hashes: [hash]`, `delete_files: bool` (required) | `{state: enum(done\|pending), undo_until: ts?}` (pending when deleting files; `undo_until` only when files are kept) | yes | P1-06, P1-09, P6-08 |
| `torrents.files` | `hash: hash` | `{files: [File]}` | no | P1-06 |
| `torrents.set_files` | `hash: hash`, `include: [u64]?`, `skip: [u64]?` | `{state: "done"}` | yes | P1-06 |
| `torrents.set_seed` | `hashes: [hash]`, `on: bool` | `{state: "done"}` | yes | P1-06 |
| `torrents.set_upload_limit` | `hashes: [hash]`, `bps: bps?` (≥ 16384, or `null` for unlimited) | `{state: enum(done\|pending)}` (pending until P6-07) | yes | P1-06 |
| `torrents.open_target` | `hash: hash`, `file: u64?` | `{path: str, kind: enum(file\|folder)}` | no | P1-06 |
| `torrents.recheck` | `hashes: [hash]` | `{state: "pending"}` | yes | P5-03 |
| `torrents.peers` | `hash: hash` | `{peers: [Peer]}` | no | P5-01 |
| `torrents.trackers` | `hash: hash` | `{trackers: [Tracker]}` | no | P6-01 |
| `torrents.reannounce` | `hashes: [hash]` | `{state: "done"}` | yes | P6-01 |
| `torrents.pieces` | `hash: hash` | `Pieces` | no | P6-02 |
| `torrents.force_start` | `hashes: [hash]`, `on: bool` | `{state: "done"}` | yes | P6-05 |
| `torrents.queue_move` | `hash: hash`, `to: enum(top\|up\|down\|bottom)` | `{state: "done", queue_pos: u64}` | yes | P6-05 |
| `torrents.move` | `hashes: [hash]`, `save_path: str` | `{state: "pending"}` | yes | P7-02 |
| `torrents.rename` | `hash: hash`, `name: str` (1..512) | `{state: "done"}` | yes | P7-03 |
| `torrents.add_tracker`, `torrents.remove_tracker` | `hash: hash`, `url: str` | `{state: "done"}` | yes | P7-04 |
| `torrents.set_download_limit` | `hashes: [hash]`, `bps: bps?` | `{state: "done"}` | yes | P7-06 |

### A.6 Operations: `search`, `rss`, `shows`, `imdb`, subscriptions

| Op | Args | Result | Mutating | Since |
|---|---|---|---|---|
| `search.providers` | none | `{providers: [Provider]}` | no | P3-01 |
| `search.set_provider` | `name: str`, `enabled: bool` | `{}` | yes | P3-01 |
| `search.install` | `curated: bool?` or `sources: [str]?` (https URLs) | `{state: "pending"}` then `mutation.result` with `{installed: [str], failed: [{source, message}]}` | yes | P3-01 |
| `search.start` | `query: str` (1..256), `providers: [str]?`, `limit: u64?` (≤ 5,000) | `{search_id: u64}`; results on `search:<id>` | no (creates a session) | P3-01 |
| `search.stop` | `search_id: u64` | `{}` | no | P3-01 |
| `search.browse_sources` | none | `{sources: [{id: str, label: str, provider: str, in_app: bool, url: str?}]}` | no | P3-01 |
| `search.browse` | `source: str` | `{items: [SearchResult]}` | no | P3-01 |
| `search.grab` | `search_id: u64` and `key: str`, or `result: SearchResult`; plus `torrents.add`'s optional args | as `torrents.add`, with `origin: "search"` | yes | P3-01 |
| `rss.feeds` | none | `{feeds: [Feed]}` | no | P3-02 |
| `rss.add` | `url: str`, `name: str?` | `{feed: Feed}` | yes | P3-02 |
| `rss.remove` | `feed: str` (id) | `{}` | yes | P3-02 |
| `rss.refresh` | `feed: str?` | `{state: "pending"}` | no | P3-02 |
| `rss.items` | `feed: str`, `unread_only: bool?` | `{items: [FeedItem]}` | no | P3-02 |
| `rss.set_read` | `feed: str`, `item: str`, `read: bool` | `{}` | yes | P3-02 |
| `rss.mark_feed_read` | `feed: str` | `{count: u64}` | yes | P3-02 |
| `rss.grab` | `feed: str`, `item: str`; plus add args | as `torrents.add`; marks read only on success | yes | P3-02 |
| `shows.list` | none | `{shows: [Show]}` | no | P3-03 |
| `shows.follow` | `query: str` or `imdb: str`, `quality: str?` | `{show: Show}` | yes | P3-03 |
| `shows.unfollow` | `imdb: str` | `{}` | yes | P3-03 |
| `shows.set_quality` | `imdb: str`, `quality: enum(2160p\|1080p\|720p\|480p)?` (`null` = default) | `{show: Show}` | yes | P3-18 |
| `shows.check` | `imdb: str?` | `{state: "pending"}` then `mutation.result` with `{new: u64}` | no | P3-03 |
| `shows.releases` | `imdb: str`, or `inbox: true`; `unread_only: bool?`, `quality: str?` | `{releases: [Release]}` | no | P3-03, P3-18 |
| `shows.set_read` | `imdb: str`, `key: str`, `read: bool` | `{}` | yes | P3-03 |
| `shows.mark_show_read` | `imdb: str` | `{count: u64}` | yes | P3-03 |
| `shows.undo_read` | none | `{restored: {imdb, key}?}` | yes | P3-03 |
| `shows.grab` | `imdb: str`, `key: str`; plus add args | as `torrents.add`, `origin: "shows"` | yes | P3-03 |
| `imdb.lookup` | `name: str` or `id: str`, `refresh: bool?` (re-download the ratings index and reload it first; `imdb --refresh`) | `{card: ImdbCard?}` | no (`refresh` writes the cache) | P3-04, P3-07 |
| `subscribe` | `topics: [str]` (1..16, the per-connection bound of §3.4.10), `interval_ms: u64?` (250..10000; applies to every topic of the call and is ignored by the "on change" topics of A.7; when omitted, each topic uses its A.7 default), `since_seq: u64?` (only with the `events` topic, A.7) | `{sub: u64, topics: [str]}` | no | P1-08 |
| `unsubscribe` | `sub: u64` | `{}` | no | P1-08 |

### A.7 Topics

| Topic | Kind | Key | Item type | Default interval | Used by |
|---|---|---|---|---|---|
| `summary` | state | — | `Summary` | 5,000 ms idle bar; 1,000 ms open | bar, popup, dashboard, TUI |
| `torrents` | keyed | hash | `Torrent` | 1,000 ms | popup (open), dashboard, TUI |
| `torrent:<h>.files` | keyed | `index` | `File` | 1,000 ms | details |
| `torrent:<h>.peers` | keyed | `addr` | `Peer` | 1,000 ms | details (P5) |
| `torrent:<h>.trackers` | keyed | `url` | `Tracker` | 5,000 ms | details (P6) |
| `torrent:<h>.pieces` | state | — | `Pieces` | 2,000 ms | details (P6) |
| `network` | state | — | `NetworkState` | on change | every UI |
| `settings` | state | — | `Settings` | on change | dashboard, TUI |
| `stats` | state | — | `Stats` | 5,000 ms | dashboard (P6) |
| `log` | keyed, append-only | `seq` | `LogEntry` | on change | dashboard, TUI, `log --follow` |
| `rss.feeds` | keyed | `id` | `Feed` | on change | dashboard, TUI |
| `rss.items:<feed>` | keyed | `id` | `FeedItem` | on change | dashboard, TUI |
| `shows` | keyed | `imdb` | `Show` | on change | dashboard, TUI |
| `shows.releases:<imdb>` or `shows.releases:inbox` | keyed | `key` (`<imdb>/<key>` in the inbox) | `Release` | on change | dashboard, TUI |
| `search:<id>` | keyed | `key` | `SearchResult` | 500 ms | search views |
| `events` | keyed, append-only | `seq` | the unsolicited events of A.8 (`torrent.*`, `engine.changed`, `mutation.result`, `search.done`, `shows.checked`, `clipboard.magnet`) wrapped as `{seq, ts, ev, …}` | on change | `events --follow`, agents |

Subscribing to `events` with `since_seq: u64?` replays the retained ring from that sequence (the
snapshot), then streams live. The ring keeps 1,000 entries or 10 minutes, whichever is smaller;
a `since_seq` older than the ring answers `snapshot.begin {reason: "resync", oldest_seq}` (A.8;
the `subscribe` result itself is unchanged) so the client knows it missed events. `seq` restarts at 1 with
every `instance`, so a client stores `(instance, seq)`.

Subscribing to an unknown topic is `invalid_args` with `details.key = "topics"`; to a
`torrent:<h>.*` topic of an unknown hash, `not_found`.

### A.8 Events

| Event | Fields |
|---|---|
| `snapshot.begin` | `sub: u64`, `topic: str`, `rev: u64`, `count: u64`, `reason: enum(subscribe\|engine_restart\|resync)`, `oldest_seq: u64?` (only with `reason: resync` on the `events` topic, A.7) |
| `snapshot.chunk` | `sub`, `topic`, `rev`, `items: [object]` (≤ 256) |
| `snapshot.end` | `sub`, `topic`, `rev` |
| `delta` | `sub`, `topic`, `rev`, `changed: {key: {field: value \| null}}`, `added: [object]`, `removed: [key]` (each may be absent when empty) |
| `state` | `sub`, `topic`, `rev`, `data: object` |
| `engine.changed` | `epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `reason: open(start\|settings\|network\|restart\|upgrade)` |
| `torrent.added` | `hash`, `name`, `origin` |
| `torrent.completed` | `hash`, `name`, `open_target: str` |
| `torrent.playable` | `hash`, `name`, `file: u64`, `open_target: str`, `buffered_s: u64?` |
| `torrent.error` | `hash`, `name`, `message: str` |
| `mutation.result` | `ref: str`, `op: str`, `outcome: enum(done\|failed\|timeout)`, `error: Error?`, `result: object?` |
| `search.done` | `search_id: u64`, `total: u64`, `warnings: [{provider: str, message: str}]` |
| `shows.checked` | `imdb: str?`, `new: u64`, `warnings: [{imdb, message}]` |
| `clipboard.magnet` | `hash`, `name: str?`, `already_added: bool` |
| `heartbeat` | `ts: ts` |
| `bye` | `reason: enum(shutdown\|idle\|restart\|upgrade\|slow_client\|hello_required\|invalid_message)` |
| `error` | `error: Error` (A.3); only for errors that have no request to answer (A.1); always followed by `bye` |

Unsolicited events (`torrent.*`, `engine.changed`, `mutation.result`, `search.done`,
`shows.checked`, `clipboard.magnet`, `heartbeat`, `bye`) are sent to every connection that has
completed `hello`, whether or not it subscribed to a topic; they are not coalesced and do not carry
`rev`. Every unsolicited event except `heartbeat` and `bye` carries `seq: u64` (daemon-global,
monotonic per instance) and `ts`, and is also appended to the `events` topic (A.7) so a client can
replay what it missed with `since_seq`.

### A.9 Objects

| Object | Fields (`?` = absent when unknown or not applicable) |
|---|---|
| `Torrent` | `hash`, `name: str (≤ 1,024 in lists)`, `state: enum(metadata\|checking\|downloading\|stalled\|queued\|paused\|done\|seeding\|waiting_vpn\|error)`, `reason: open(user\|queue_full\|complete_no_seed\|vpn_down\|restore_failed\|metadata_timeout\|missing_files\|disk_full)?`, `error: str?`, `intent: enum(run\|pause)`, `staged: bool`, `checked: bool`, `progress: f64?`, `size: bytes?`, `completed_bytes: bytes?`, `uploaded_bytes: bytes?` (this torrent's upload in the current engine life, from `stats.live`), `dl_bps?`, `ul_bps?`, `eta_s: u64?`, `peers: u64?`, `seeds: u64?`, `seed: bool`, `upload_policy: enum(default_cap\|unlimited\|custom)`, `upload_limit_bps?`, `ratio: f64?`, `save_path: str`, `content_path: str?`, `added_at: ts`, `completed_at: ts?`, `origin: open(…)?`, `imdb_id: str?`, `queue_pos: u64?`, `force: bool`, `playable: Playable?` |
| `TorrentDetail` | every `Torrent` field plus `file_count: u64?`, `piece_count: u64?`, `piece_size: bytes?`, `comment: str?`, `created_by: str?`, `private: bool`, `tracker_count: u64?`, `undo_until: ts?` |
| `Playable` | `file: u64`, `ready: bool` (the DL-26 threshold), `stall_free: bool?` (at the current `dl_bps`, the rest of the file arrives before playback reaches the end of the prefix: `prefix_bytes / bitrate ≥ (file_size − prefix_bytes) / dl_bps`; absent when no duration is known or the torrent is not downloading), `prefix_bytes: bytes`, `buffered_s: u64?` (`prefix_bytes / bitrate`, with `bitrate = file_size / duration`), `finishes_in_s: u64?` (`(file_size − prefix_bytes) / dl_bps`, the right-hand side of the rule; absent when `dl_bps` is absent or 0), `acked: bool` (§4.5 icon acknowledgement) |
| `File` | `index: u64`, `path: str`, `size: bytes`, `progress: f64`, `included: bool`, `sort_key: str` (computed by the daemon in `torrents.files` and the files topic, P1-06: the path lowercased with every run of ASCII digits left-padded with zeros to 10 digits, so that a plain string comparison yields natural order: `e2.mkv` < `e10.mkv`), `ready: bool?` (P5-05) |
| `Peer` | `addr: str`, `client: str?`, `conn: enum(tcp\|tcp_mse)`, `incoming: bool`, `state: open(connecting\|live\|dead\|queued)`, `downloaded: bytes`, `uploaded: bytes`, `dl_bps?`, `ul_bps?`, `seed: bool?` |
| `Tracker` | `url: str`, `tier: u64?`, `status: enum(ok\|error\|updating\|not_contacted)`, `last_announce_at: ts?`, `next_announce_at: ts?`, `peers: u64?`, `message: str?` |
| `Pieces` | `count: u64`, `piece_size: bytes`, `have: str` (the have-bitfield, MSB-first as in BEP 3, base64; `ceil(count/8)` bytes before encoding, so 100,000 pieces are 16.7 KB and the frame is bounded by construction, which a run-length form is not), `priority: [u64]` (first and last pieces per file), `playable_end: u64?` |
| `Summary` | `dl_bps?`, `ul_bps?`, `counts: {total, downloading, stalled, metadata, checking, queued, paused, done, seeding, waiting_vpn, error, staged, ready}` (all `u64`), `alt: bool`, `limits: Limits`, `network: {state: enum(ready\|waiting\|off), message: str, interface: str?}`, `daemon: {state: enum(starting\|running\|parked\|stopping), idle_exit_in_s: u64?, busy_reasons: [open(downloading\|seeding\|parked_waiting\|interactive\|search\|mutation)], upgrade_pending: bool}`, `top: [{hash, name, state, progress?, dl_bps?, eta_s?, playable?}]` (≤ 5), `disk: {free_bytes: bytes?, needed_bytes: bytes, low: bool}` (P5-07; the one place free space lives), `history: [u64]?` (last 60 download samples, P6-03) |
| `Limits` | `download_bps?`, `upload_bps?`, `alt_download_bps?`, `alt_upload_bps?` |
| `NetworkState` | `selection: str?`, `interface: str?`, `ready: bool`, `message: str` (from `src/network_app.rs:13-20`) |
| `Settings` | `max_active_downloads: u64`, `peer_encryption: enum(off\|prefer\|require)`, `network_interface: str?`, `save_path: str`, `seed_by_default: bool`, `autostart: bool`, `idle_exit_secs: u64` (0 disables the idle net, §3.2.10), `notify: bool`, `timeout_secs: u64`, `handler: enum(add\|confirm)`, `open_command: str` (default `xdg-open`, §6.2 row 40), `clipboard_watch: bool`, `peer_port: u64`, `show_quality: str`, `download_limit_bps?`, `upload_limit_bps?`, `alt_download_limit_bps?`, `alt_upload_limit_bps?`, `alt_limits_on: bool` (the persisted turtle switch; `transfer.set_alt` writes it), `restart_class: [str]` (the keys whose change restarts the engine). Every key is also the `config.toml` key: one vocabulary for the file, the API and `fbittorrent settings` |
| `Stats` | `session: {downloaded: bytes, uploaded: bytes, started_at: ts}`, `all_time: {downloaded: bytes, uploaded: bytes, completed: u64}`, `history: {download: [u64], upload: [u64], interval_s: u64}?` |
| `LogEntry` | `seq: u64`, `ts: ts`, `level: enum(error\|warn\|info\|debug)`, `source: open(daemon\|engine\|queue\|api\|search\|rss\|shows\|imdb\|notify\|network\|watch\|clipboard)`, `hash?`, `message: str` |
| `Provider` | `name: str`, `full_name: str?` (the nova plugin's display name, today's `fullName`), `kind: enum(nova\|torznab)`, `enabled: bool`, `url: str?`, `version: str?` |
| `SearchResult` | `key: str`, `name: str`, `size: bytes?`, `seeds: u64?`, `leechers: u64?`, `uploaded_at: ts?`, `providers: [str]`, `magnet: str?`, `url: str?`, `page_url: str?`, `imdb_id: str?`, `rating: f64?`, `provider_url: str?` (the first provider's site, today's `siteUrl`), `added: hash?` (the ledger entry with the same btih, when one exists; the row shows `+ In your list` and Enter selects that torrent instead of adding; the daemon fills it from the ledger as results stream, so no client needs the ledger) |
| `Feed` | `id: str` (`rss::stable_id(url)`, `src/rss.rs:175`, today's `uid`), `name: str`, `url: str`, `title: str?`, `unread: u64`, `last_refresh_at: ts?`, `loading: bool`, `error: str?` |
| `FeedItem` | `id: str`, `title: str`, `description: str?`, `published_at: ts?`, `size: bytes?`, `link: str?`, `magnet: str?`, `torrent_url: str?`, `read: bool`, `grabbed: bool` |
| `Show` | `imdb: str`, `title: str`, `year: u64?`, `quality: str?` (per-show override), `unread: u64`, `added_at: ts`, `counts: {episodes: u64, seen: u64, grabbed: u64}`, `last_check_at: ts?`, `checking: bool`, `catalogue_complete: bool`, `warning: str?` |
| `Release` | `key: str` (`S02E03` or `S02`), `show_imdb: str`, `title: str`, `season: u64?`, `episode: u64?`, `season_pack: bool`, `quality: str?`, `seeds: u64?`, `size: bytes?`, `magnet: str?`, `candidates: u64?`, `published_at: ts?`, `read: bool`, `transfer: enum(none\|downloading\|queued\|paused\|done\|added_before)` |
| `ImdbCard` | `id: str`, `title: str`, `year: u64?`, `kind: str`, `rating: f64?`, `votes: u64?`, `stars: str?`, `blurb: str?`, `wiki_url: str?`, `url: str`, `poster_url: str?`, `poster_path: str?` |

### A.10 Field provenance for `Torrent`

| Field | Source today |
|---|---|
| `hash`, `name`, `save_path`, `added_at`, `completed_at`, `intent`, `upload_limit_bps`, `origin` (new), `force` (new) | `Record` (`src/engine.rs:38-53`: `hash, name, source, output_folder, only_files, upload_limit, ratio_limit, category, tags, added_on, completion_on, paused`) |
| `state`, `reason` | §3.5 from `TorrentStats.state`, `finished`, `is_paused`, `record.paused`, `stop_on_complete` (`src/engine.rs:1396-1440`) |
| `progress`, `size`, `completed_bytes` | `stats.progress_bytes / total_bytes` (`src/engine.rs:1397-1404`) |
| `dl_bps`, `ul_bps`, `peers` | `stats.live` (`src/engine.rs:1405-1415`) |
| `seeds` | P5-01 |
| `eta_s` | `(total − progress) / dl_bps` when `dl_bps > 0` (`src/engine.rs:1474-1478`, minus the sentinel) |
| `ratio` | `uploaded / progress_bytes` when `progress_bytes > 0` (`src/engine.rs:1482-1486`) |
| `seed`, `upload_policy` | `upload_limit == 0 && ratio_limit == -2` → `unlimited`; `== 32768 && 0.0` → `default_cap`; else `custom` (`src/qbt.rs:504-520`) |
| `content_path` | `qbt::open_target` (`src/qbt.rs:706`) |
| `checked` | P1-16 |
| `playable` | P5-05 |
| `queue_pos` | ledger order (P6-05) |

### A.11 Example: the bar and the popup

```
→ {"id":1,"op":"hello","args":{"proto":1,"client":{"name":"oliver.torrents","version":"0.2.0","kind":"bar"}}}
← {"id":1,"ok":true,"result":{"proto":1,"state":"running","caps":["torrents","files","subscribe","mutations","log"], …}}
→ {"id":2,"op":"subscribe","args":{"topics":["summary"],"interval_ms":5000}}
← {"id":2,"ok":true,"result":{"sub":1,"topics":["summary"]}}
← {"ev":"state","sub":1,"topic":"summary","rev":1,"data":{"dl_bps":4123456,"ul_bps":32768,
     "counts":{"total":12,"downloading":2,"queued":1,"done":8,"ready":0,…},"alt":false,
     "daemon":{"state":"running","busy_reasons":["downloading"],"upgrade_pending":false}, …}}
   (the popup opens)
→ {"id":3,"op":"subscribe","args":{"topics":["torrents"],"interval_ms":1000}}
← {"id":3,"ok":true,"result":{"sub":2,"topics":["torrents"]}}
← {"ev":"snapshot.begin","sub":2,"topic":"torrents","rev":1,"count":12,"reason":"subscribe"}
← {"ev":"snapshot.chunk","sub":2,"topic":"torrents","rev":2,"items":[{"hash":"3f2a…","state":"downloading", …}, …]}
← {"ev":"snapshot.end","sub":2,"topic":"torrents","rev":3}
→ {"id":4,"op":"torrents.stop","args":{"hashes":["3f2a…"]},"ref":"qs-1729-1"}
← {"id":4,"ok":true,"result":{"state":"done","stopped":["3f2a…"]}}
← {"ev":"delta","sub":2,"topic":"torrents","rev":4,
     "changed":{"3f2a…":{"state":"paused","reason":"user","dl_bps":null,"eta_s":null}}}
   (the popup closes)
→ {"id":5,"op":"unsubscribe","args":{"sub":2}}
← {"id":5,"ok":true,"result":{}}
```

### A.12 Example: an agent waits for playable

```
$ fbittorrent add "magnet:?xt=urn:btih:…" --json
{"ok":true,"action":"add","hash":"3f2a…","name":"Sintel.2010.1080p","source":"magnet","sequential":true,"first_last":true,"duplicate":false}
$ fbittorrent wait 3f2a --until playable --timeout 600 --json
{"ok":true,"complete":false,"playable":true,"torrents":[{"hash":"3f2a…","playable":{"file":0,"ready":true,"prefix_bytes":41943040,"buffered_s":212},"open_target":"/home/oliver/Downloads/Sintel.2010.1080p.mkv"}]}
$ fbittorrent events --follow --kinds torrent.completed --json
{"ev":"torrent.completed","hash":"3f2a…","name":"Sintel.2010.1080p","open_target":"/home/oliver/Downloads/Sintel.2010.1080p.mkv"}
```

---
## Appendix B. Keymap

"New" marks keys this plan adds. Everything else exists today and keeps its meaning. The table is
generated from the action table after P4-02 (`fbittorrent robot-docs keys`); until then it is
hand-maintained here and in `plugin/oliver.torrents/keymap.json`.

**Keymap schema** (one shape for `keymap.json` and `robot-docs keys --json`, P4-02): `{"version":
1, "actions": [ … ]}`, the array in this table's row order; each element is `{"id":
"torrents.remove", "label": "Remove, keep files", "scope": "screen:torrents" | "global" |
"pane:<screen>.<pane>" | "modal:<kind>" | "prompt", "keys": {"tui": ["d", "x", "Delete"],
"dashboard": ["d", "x", "Delete"], "popup": ["d", "x", "Delete"]}, "note": str?}`; a surface that
does not offer the action has `null` in `keys`; `note` carries a surface exception (the popup's
"Enter opens the file"); keys are spelled as in this table (`Ctrl+k`, `Shift+Tab`, `F6`, `Space`,
`Enter`, `Esc`). The shape is action-keyed (one element per row of this table) rather than
surface-keyed so that `no_duplicate_key_per_scope`, the README table and this appendix are all the
same rows; `robot-docs keys --surface <s>` prints the same elements with the other surfaces'
`keys` omitted. P4-02 generates exactly this shape from the action table, so P3-15's hand-written
first version needs no migration.

| Action | TUI | Dashboard | Popup |
|---|---|---|---|
| Move | `j`/`k`, ↑/↓ | `j`/`k`, ↑/↓ | `j`/`k`, ↑/↓ |
| Page | PgUp/PgDn (new on lists) | PgUp/PgDn | n/a |
| First / last | `g`/`G`, Home/End | `g`/`G`, Home/End | n/a |
| Switch screen | `1`-`4`, Tab/Shift+Tab | `1`-`4`, Tab/Shift+Tab | Tab switches Omarchy panels |
| Pane focus | `h`/`l`, ←/→ | `h`/`l`, ←/→ | n/a |
| Help | `?` | `?` | n/a |
| Filter or search | `/` | `/` | `/` opens the dashboard on Search |
| Back one layer | Esc | Esc | Esc closes |
| Log (new) | `L` | `L` | n/a |
| Settings | F6 (`,` new alias) | `,` | n/a |
| Turtle limits (new, P5-04) | `z` | `z` | `z` |
| Close this UI | `q`, Ctrl+C (no longer stops downloads) | `q`, or close the window; Esc at the top level clears the filter and the selection and then does nothing (a long-lived window must not vanish on a reflex key) | Esc, `q` |
| Stop fbittorrent (the daemon and everything in it, A1) | `Q` (confirms when active) | `Q`, Ctrl+Q | `Q` or the power icon `󰐥` (confirms when active) |
| Torrent: primary action | Enter opens details (Overview first); double-click too | Enter opens details; double-click too | Enter opens the file (exception 1) |
| Open file or folder | `o` | `o` | `o` |
| Pause / resume | `p`, Space | `p`, Space | `p`, Space |
| Seed on/off | `u` | `u` | `u` |
| Remove, keep files | `d`, `x` (new alias), Delete | `d`, `x`, Delete | `d`, `x`, Delete (confirm strip) |
| Remove and delete files | `D` | `D` | not offered |
| Undo last remove (new, P6-08) | `U` while the undo toast shows (never `u`, which is seed on the same screen; a `u` meant for the next row must not silently re-add a torrent) | the toast's button, `U` | n/a |
| Bulk remove the filtered set (new, P6-08) | `X` | `X`, bound through a `Shortcut { sequence: "Shift+X"; context: Qt.WindowShortcut }` in `Dashboard.qml`, because `PanelKeyCatcher` folds `x`, `X` and Delete into one `deleteRequested()` with no modifier (`Ui/PanelKeyCatcher.qml:93-95`) | n/a |
| Add | `a` prompt (F4 seed toggle), `c` from clipboard | `a` or Ctrl+V: add sheet | `a` or right click: the offer strip with the clipboard's torrent (§4.8); Enter adds |
| Recheck (new, P5-03) | `R` (list; also in the details modal) | `R` (list and details pane); the Overview "Recheck…" button | n/a |
| Reannounce (new, P6-01) | `e` in the Trackers tab of the details modal | `e` in the Trackers tab | n/a |
| Force start (new, P6-05) | `P` | `P` | n/a |
| Queue up / down (new, P6-05) | `K`/`J` | `K`/`J`, Ctrl+k/Ctrl+j and Ctrl+↑/Ctrl+↓ (`PanelKeyCatcher`'s `reorderRequested`, `Ui/PanelKeyCatcher.qml:39-42,64-70`) | n/a |
| Sort (new) | `s` on every list after P4-09 (today `s` is a legacy-only toggle): Torrents by Added, Name, Progress, ↓, Size, State, ETA, Queue (P6-05); Search by Seeds, Size, Uploaded, Name, ★; Shows releases by Date, Seeds, Size; `S` reverses | `s`, `S` reverses, header click | n/a |
| Follow show | `f` (new on Torrents), `F` | `f` | n/a |
| Show quality (new, P3-18) | `=` on a show row (a prompt); `Q` stays quit everywhere | the dropdown on the show row | n/a |
| IMDb / Wikipedia | `i` / `w` | `i` / `w` | n/a |
| Open dashboard / TUI | n/a | n/a | `w` / `t` |
| Details tabs (new) | `[` / `]` | `[` / `]` | n/a |
| Poster card at narrow widths (new, P4-11) | `v` | `v` toggles grid/rows on Browse | n/a |
| Files: toggle / include / all / none | Space / `1` / `a` / `n` | Space / n/a / `a` / `n` | n/a |
| Search: focus query | `/`, `e` | `/` | n/a |
| Search: run, or download the result | Enter | Enter (stages the add) | n/a |
| Search: browse lists / providers / source page | `b` / `P` / `O` (`P` has no force-start meaning on Search; `s` is sort there as everywhere) | `b` / `P` / `O` | n/a |
| Copy magnet (new) | `y` on a torrent or a search result in list scope (`wl-copy`; omaqbt's `y` on detail, ucmz851's one-click copy); inside a confirm prompt `y` still confirms, which is a different scope | `y` | n/a |
| Shows: download release, or enter releases | Enter | Enter | n/a |
| Shows: toggle read / all read / undo | `m`, Space / `M` / `u` | `m` / `M` / `u` | n/a |
| Shows: check / follow / unfollow / source | `r`, `R` / `n`, `+` / `x` (confirm) / `O` | same; `d` also unfollows | n/a |
| RSS: open items, or download item | Enter | Enter (stages the add) | n/a |
| RSS: toggle read / all read | `m`, Space / `M` | `m` / `M` | n/a |
| RSS: refresh / new feed / remove feed | `r`, `R` / `n` / `x` (confirm) | same; `d` also removes | n/a |
| Clipboard offer (new, P6-13): add / dismiss | `y` / Esc while the strip shows | the strip's buttons, `y` / Esc | `y` / Esc |
| Confirm a destructive prompt | `y` only | `y`, or the focused button | `y`, or click |
| Cancel a prompt | Esc, Enter, any other key | Esc, Enter on Cancel (focused by default) | Esc |

Two exceptions to "same letters everywhere", both in the popup and both listed only here: (1) Enter
opens the file (today's behaviour, `Panel.qml:72-76`) where the dashboard's and the TUI's Enter
open details; (2) `w` opens the dashboard window where the dashboard's and the TUI's `w` open
Wikipedia (the popup has no IMDb card, so the letter is free there). `q` is not an exception: it
closes the surface it is pressed in, everywhere, and `Q` quits everywhere (today's popup treats
`q` and `Q` alike and stops the daemon, `Panel.qml:179`; that changes in P2-03).
(Space is not an exception: `PanelKeyCatcher` lets a panel tell Enter from Space through
`returnRequested`, `Ui/PanelKeyCatcher.qml:86-92`, so Space pauses everywhere; §4.12.)

`u` means "seed" on Torrents and "undo the last inbox read" in the Shows inbox, where there is
nothing to seed; `U` undoes a removal while its toast shows, on every surface. `Q` means "quit"
everywhere; show quality is `=`, so the quit verb never changes meaning by screen (`src/tui/mod.rs:1207-1212`
is the owner's muscle memory). `s` sorts on every list; Search's providers sheet is `P`. P4-02's
`no_duplicate_key_per_scope` treats "while the toast shows" as the same scope as the screen, so
the `u`/`U` split is machine-checked. §10.3 Q14 asks the owner to confirm `U` and `=`.

Until P4-09 removes legacy mode, `s` and `f` on the Torrents screen keep their legacy-only
meanings (sequential and first/last toggles, `src/tui/mod.rs:1331-1336`) when `engine = "qbittorrent"`;
in native mode they are free, which is why `s` (sort) and `f` (follow) can take them.

---

## Appendix C. More screen mockups

**Popup, idle with a ready-to-watch row (owner's theme; keyboard mode, so the KeyHint row shows):**

```
┌────────────────────────────────────────────────────┐
│ 󰇚  Torrents                          󰐕  ▢  󰢃  󰐥    │
│    ↓ 2.0 MB/s  ↑ 32 KB/s             1 DOWNLOADING │
│ ────────────────────────────────────────────────── │
│ DOWNLOADING                                        │
│ ▶ Some.Movie.2026.1080p.WEB-DL…                    │
│   ▕████▌                        ▏                  │
│   Ready to watch · 18% · 4 min buffered   [Open]   │
│ DONE 8                                        ›    │
│ Ready to watch · press o to start the player       │
│ ────────────────────────────────────────────────── │
│ a add · / search · w window · t terminal · z turtle│
└────────────────────────────────────────────────────┘
```

**Popup, waiting for the VPN, with a confirm strip open (keyboard mode):**

```
┌────────────────────────────────────────────────────┐
│ 󰦝  Torrents                          󰐕  ▢  󰐥       │
│    Waiting for ProtonVPN            2 WAITING      │
│ ────────────────────────────────────────────────── │
│ ▲ Downloads resume when proton0 connects.          │
│ ────────────────────────────────────────────────── │
│ 󰦝 The Diplomat S04E01 1080p…                       │
│   Waiting for VPN · progress unknown               │
│   Remove "The Diplomat S04E01…"? Files are kept.   │
│   [ Cancel ]  [ Remove ]                           │
│ ────────────────────────────────────────────────── │
│ a add · / search · w window · t terminal           │
└────────────────────────────────────────────────────┘
```

**Popup, daemon not running (with `showWhenStopped`):**

```
┌────────────────────────────────────────────────────┐
│ 󰕒  Torrents                                        │
│    NOT RUNNING                                     │
│ ────────────────────────────────────────────────── │
│   fbittorrent is not running.                      │
│   Enter Start   ·   w Open window                  │
└────────────────────────────────────────────────────┘
```

**Dashboard, Search view with the poster grid (empty query, `v` grid):**

```
┌ fbittorrent ──────────────────────────────────────────────────────────────────────────────────────────────────┐
│ 1 Torrents  2 Search  3 Shows ●4  4 RSS ●2      [                        ]  ↓ 4.2M  󰦝 proton0                 │
├───────────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ YTS · Latest movies ▾           rows v                                                                        │
│ ┌──────┐  ┌──────┐  ┌──────┐  ┌──────┐  ┌──────┐  ┌──────┐                                                    │
│ │      │  │      │  │      │  │      │  │      │  │      │                                                    │
│ │poster│  │poster│  │poster│  │poster│  │poster│  │poster│                                                    │
│ │      │  │      │  │      │  │      │  │      │  │      │                                                    │
│ └──────┘  └──────┘  └──────┘  └──────┘  └──────┘  └──────┘                                                    │
│ Sintel     Big Buck  Tears of  Cosmos    Elephants Spring                                                     │
│ 2010 ★7.4  2008 ★7.4 2012 ★6.6 2019 ★7.0 2006 ★7.1 2019 ★6.8                                                  │
│ 812 seeds  1.2k      96        44        210       312                                                        │
├───────────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ Enter add  i IMDb  v rows  b lists  P providers  / search  Esc back                                           │
└───────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

**Dashboard, Search view (rows):**

```
┌ fbittorrent ──────────────────────────────────────────────────────────────────────────────────────────────────┐
│ 1 Torrents  2 Search  3 Shows ●4  4 RSS ●2      [sintel                  ]  ↓ 4.2M  󰦝 proton0                 │
├────────────────────────────────────────────────────────────────────────────────────┬──────────────────────────┤
│ NAME                                  SIZE  SEEDS  UPLOADED  FROM           ★      │ ┌───────┐ Sintel         │
│ ▸ Sintel 2010 1080p BluRay x264       1.4G    812  2011-02   tpb, yts     7.4      │ │poster │ (2010)         │
│   Sintel 2010 4k FLAC mkv             6.1G     96  2019-06   bitsearch    7.4      │ └───────┘ ★ 7.4          │
│   Sintel.2010.720p.WEB                690M     41  2015-01   eztv         7.4      │ Short film by the        │
│   … 84 more · 3 providers still searching                                          │ Blender Institute        │
├────────────────────────────────────────────────────────────────────────────────────┴──────────────────────────┤
│ Enter add  f follow  i IMDb  O source  b browse  P providers  Esc back                                        │
└───────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

**Dashboard at 900 px (chip-row tier, §4.3.3: SIZE, PROGRESS, ↓, ETA, PEERS; NAME ≥ 352 px; details as a sheet):**

```
┌ fbittorrent ───────────────────────────────────────────────────────────────────────────┐
│ 1 Torrents  2 Search  3 Shows ●4  4 RSS ●2   [/ filter   ]  ↓ 4.2M  󰢃                  │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ All 12 · Downloading 2 · Ready 1 · Queued 1 · Paused 1 · Done 8                        │
│ STATE        NAME                            SIZE  PROGRESS        ↓     ETA  PEERS    │
│ 󰇚 Downloading The Diplomat S04E01 1080p WE…  1.1G ▕███▌   ▏43%   3.1M   12m   41       │
│ ▶ Ready       Some.Movie.2026.1080p          2.1G ▕█▌     ▏18%   2.0M   18m   12       │
│ … Metadata    Other.Movie.2026.1080p          —   ▕       ▏ —     —      —    3        │
│ ┌─ The Diplomat S04E01 1080p WEB-DL H264-GROUP ────────────────────────────── Esc ─┐   │
│ │ Overview  Files  Peers  Trackers  Pieces  Log                                    │   │
│ │ Downloading · 43% · 486 MiB of 1.1 GiB · ↓ 3.1 MB/s · 12 min · 41 peers          │   │
│ │ ☑ S04E01.mkv  1.1 GiB  43%                                                       │   │
│ └──────────────────────────────────────────────────────────────────────────────────┘   │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ Enter details  p pause  u seed  d remove  a add  o open  ? keys                        │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

**Dashboard, remove dialog (`ConfirmDialog`, `selectedIndex: 0`):**

```
          ┌──────────────────────────────────────────────────────┐
          │ Remove "Sintel.2010.1080p"?                          │
          │ 1.4 GiB in ~/Downloads/Sintel.2010.1080p             │
          │                                                      │
          │   [ Cancel ]   [ Remove, keep files ]                │  focus starts on Cancel
          └──────────────────────────────────────────────────────┘
          ┌──────────────────────────────────────────────────────┐
          │ Delete "Sintel.2010.1080p" and its files?            │
          │ 1.4 GiB will be deleted from ~/Downloads. This       │
          │ cannot be undone.                                    │
          │   [ Cancel ]   [ Delete files ]                      │  urgent colour
          └──────────────────────────────────────────────────────┘
```

**Dashboard, add sheet:**

```
          ┌──────────────────────────────────────────────────────┐
          │ Add torrent                                          │
          │ The.Diplomat.S04.1080p.WEB-DL        8.9 GiB · 8 files│
          │ ☑ S04E01.mkv  1.1 GiB     ☑ S04E05.mkv  1.1 GiB      │
          │ ☑ S04E02.mkv  1.1 GiB     ☑ S04E06.mkv  1.1 GiB      │
          │ ☑ S04E03.mkv  1.1 GiB     ☐ sample.mkv   40 MiB      │
          │ Seeding  ( off )  upload capped at 32 KiB/s, stops   │
          │                   when complete                      │
          │ Save to  ~/Downloads               412 GiB free      │
          │   [ Discard (Esc) ]            [ Download (Enter) ]  │
          └──────────────────────────────────────────────────────┘
```

**TUI Search at 80×24 (narrow), card hidden:**

```
  1 Torrents  [ 2 Search ]  3 Shows (2)    4 RSS         ↓ 0 B/s ↑ 0 B/s
╭ Search ──────────────────────────────────────────────────────────────────────╮
│ Sintel                                                                       │
╰──────────────────────────────────────────────────────────────────────────────╯
╭ Results (100) · 18/18 providers ─────────────────────────────────────────────╮
│   NAME                                              SIZE   SEEDS   ★         │
│ ▌ Sintel                                        123.3 MiB   124  7.4         │
│   Sintel - 2010 - Multisubs mkv                 441.5 MiB    29  7.4         │
│   [philosophy-raws][辛特尔][sintel][BDRIP…      963.2 MiB     1   —          │
╰──────────────────────────────────────────────────────────────────────────────╯
[Results] (Enter) Download  (v) Card  (F)ollow  (/) Query  (s) Sort  …  (?) Help
Sintel · torrentscsv, tpb · uploaded 2023-04-30
```

**TUI Shows at 80×24 (narrow), one pane with breadcrumb:**

```
  1 Torrents    2 Search  [ 3 Shows (2) ]  4 RSS         ↓ 0 B/s ↑ 0 B/s
╭ Shows › All unread · 2 releases ─────────────────────────────────────────────╮
│      RELEASE SHOW                                  QUAL      SIZE   SEEDS    │
│ ▌ ●  S05     Slow Horses (2022) · S05 1080p x265…  1080p   4.2 GiB    212    │
│   ●  S27     South Park (1997) · S27 1080p x265-…  1080p   1.4 GiB    245    │
╰──────────────────────────────────────────────────────────────────────────────╯
[Unread] (Enter) Download  (M)ark read  (u) Undo  (←) Shows  …          (?) Help
Slow Horses S05 1080p x265-ELiTE EZTV · 2026-10-16 · unread
```

---

## Appendix D. Glossary

| Term | Meaning here |
|---|---|
| Daemon | `fbittorrent serve`, the process that owns the engine and every feature service. |
| Engine | The librqbit session on the Asupersync runtime, plus the ledger and the queue. |
| Engine slot | The daemon's holder for the engine: `Starting`, `Running`, `Parked` or `Stopping`. |
| Engine epoch | A counter that increases on every engine restart inside one daemon life. |
| Instance | A random id minted at daemon start; a new instance means "resubscribe". |
| Parked | The engine is down because the bound VPN interface is not ready; saved data is still served. |
| Ledger | `~/.local/share/fbittorrent/engine/torrents.json`, the durable record of every torrent and the user's intent. |
| Intent | What the user asked for (`run` or `pause`), stored durably and separate from queue suspension. |
| Interactive client | A TUI, or a visible dashboard window. Keeps the daemon alive under the idle net (§3.2.10); the bar does not. |
| Presence connection | The TUI's API connection used only to be counted as interactive while its data still uses the compat socket (P1-15 to P4-03). |
| Staged torrent | Added for review in the add sheet; paused, outside the queue and the counts. |
| Dormant / lazy restore | A completed torrent served from the ledger without a hash check until the user acts on it (P1-16). |
| Fast resume | Trusting a saved have-bitfield on start instead of re-checking data (P6-06). |
| Ready to watch / playable | A video file whose contiguous prefix and last piece are on disk, so a player can start (P5-05). |
| Stall-free | `playable.stall_free`: at the current download rate the rest of the file arrives before playback reaches the end of the prefix (§3.5); "may stall" otherwise. |
| Glance | The bar popup being open: pauses the idle countdown, never counts as interactive, never starts the daemon (§3.2.10). |
| Disk low / disk full | `Summary.disk.low` (free space below the running downloads' remaining bytes plus 512 MiB) and `reason: disk_full` on a torrent that hit `ENOSPC` (P5-07). |
| Sequence (`seq`) | The daemon-global counter on every unsolicited event; the `events` topic replays from a `since_seq` (A.7). |
| Topic | A named stream of state a client subscribes to. |
| Snapshot / delta | The full state of a topic in chunks, then only the changes. |
| Revision (`rev`) | A per-subscription counter on every frame, used to detect gaps. |
| Cap | A feature group the daemon advertises in `hello`; clients hide what is absent. |
| `ref` | A client-chosen idempotency key on a mutation, keyed with the client name. |
| Pending / done | A mutation's state: still in progress, or applied and saved. |
| Compat socket | `engine.sock` with the old 4-byte framing, served by the daemon until v0.4.0. |
| `proto` | The API's major version (1). |
| Contract | The CLI's agent contract version (`capabilities`), 1.1.0 today, 1.2.0 after P1-11. |
| btih | The BitTorrent info hash in a magnet link; the dedupe key. |
| Turtle | Alternative (lower) global speed limits toggled with `z`. |
| Stop rule | The daemon stops when the owner stops it (the bar icon's power button, `fbittorrent down`, `Q` in the TUI) and by itself after `idle_exit_secs` with nothing busy (the idle net); there is no `stop_policy` setting (A1, §3.2.10). |
| Hover-cursor | The shell's one state for "mouse over" and "keyboard cursor on" (`Style.qml:40-44`). |
| Low-chroma theme | A theme whose red and green differ by less than 32 in every channel; semantics drop colour (§5.5.2). |
| T1 / T2 / T3 | Feature tiers: must, should, later (§6.1). |
| Case slug | The snake_case name of one end-to-end case, the `case` field of its JSONL lines and the argument of `--case` (`daemon.py`, `native.py`, `tui_smoke.py`). |
| Fixture state | A JSON recipe under `regression_tests/fixtures/states/<state>.json` that `daemon.py --serve-fixture <state>` turns into a running daemon with named torrents, one per row of `plugin/tests/VISUAL.md`. |
| Receipt | The record of an item's or milestone's verification: a section `## <ID>` in `docs/validation/<date>/README.md` (the file §8.3 keeps in git) listing the commands run with their exit codes, the `target/test-logs/` paths, and any manual observation or decision (for example the chord chosen in P2-07). Nothing else is a receipt, and no receipt carries files over 256 KB. |

---

## Appendix E. Test contract and verification scripts

### E.1 The log line

Every end-to-end step writes one line to `target/test-logs/<suite>/<case>.jsonl`:

```json
{"ts":"2026-10-18T12:41:03.117Z","suite":"daemon","case":"idle_exit_no_torrents","step":3,
 "action":"wait until summary.daemon.idle_exit_in_s == 0","expect":"daemon exits 0 within 5 s",
 "observed":"exit 0 after 2.31 s; bye{reason:idle} received","ok":true,"ms":2310,
 "artifacts":["target/test-logs/daemon/idle_exit_no_torrents.daemon.log"]}
```

Rules: `ts` is wall clock; `step` increments per case; `expect` and `observed` are human sentences
(what a reviewer reads first); `artifacts` lists files the step produced (pane captures, PNGs,
daemon logs). A case's last line has `"final": true` and the case verdict. The suite summary
(`target/test-logs/<suite>/summary.json`) counts cases and lists the slowest five.

### E.2 Scripts

| Script | Covers | Needs | Runs in CI | Added by |
|---|---|---|---|---|
| `agent_ergonomics_audit/audit/regression_tests/all.sh` | umbrella; flags; summary table | cargo, python3, tmux | yes (parts) | exists; P0-06 |
| `cargo test --locked -p fbittorrent -p rqbit-runtime` | unit; goldens; fixtures round-trip; palette; action table; store | cargo | yes | exists; P0-16, P1-05, P4-02, P4-03, P4-05 |
| `cargo test --locked -p librqbit-core -p librqbit --features librqbit/rust-tls --lib` | fork tests (seed flag, bitfield accessor, private flag, resume) | cargo | yes | P0-05, P5-01, P5-06, P6-06, P6-12 |
| `regression_tests/native.py` | isolated TUI/CLI; from P1-15 the TUI runs as a client of a disposable daemon | tmux, util-linux | yes | exists; P0-15, P1-15 |
| `regression_tests/daemon.py` | every daemon behaviour; `--systemd`; `--serve-fixture` | python3, optional user systemd | yes (`--no-systemd`) | P1-07 |
| `regression_tests/agent_contract.py` | CLI contract: envelopes, exit codes, determinism, stdout/stderr split, `events`, `api`, verbs | python3 | yes | P1-11 |
| `regression_tests/tui_smoke.py` | tmux TUI at 80×24, 120×30, 160×45 | tmux | yes | P4-10 |
| `regression_tests/vpn.py` | namespace fixtures: park, unpark, headless, deferred add | unshare | opt-in `FBT_TEST_VPN=1` | exists; P1-03, P7-09 |
| `regression_tests/resources.py` | CPU/RSS budget vs baseline | release-perf build | opt-in `FBT_TEST_RESOURCES=1` | exists; P1-08, P0-06 |
| `plugin/tests/qs-store.sh` (every `plugin/tests/` script lives outside the symlinked plugin folder, §4.1) | Store fixtures | qs | opt-in `FBT_TEST_QS=1` | P2-02 |
| `plugin/tests/qs-format.sh` | number formatting | qs | opt-in | P2-08 |
| `plugin/tests/qs-glyphs.sh` | glyph rendering | qs | opt-in | P2-04 |
| `plugin/tests/qs-smoke.sh` | plugin to daemon over a disposable runtime dir | qs, a build | opt-in | P2-06 |
| `plugin/tests/qs-dashboard.sh` | dashboard behaviour on the live shell | the live shell, wtype | opt-in | P3-08..P3-18, P5-02, P6-* |
| `plugin/tests/qs-visual.sh` | captures and pixel diffs, light theme | grim, omarchy-shell | opt-in `FBT_TEST_VISUAL=1` | P2-08 |
| `plugin/tests/lint.sh` | tokens, literals, structure (no Timer/Process outside Service.qml) | rg | yes | P2-04, P2-05 |
| `plugin/tests/contrast.py` | WCAG ratios per installed theme | python3 | yes (TOML only) | P2-08 |
| `regression_tests/widget_keys.sh` | popup keys through wtype and the IPC hooks | the live shell, wtype | opt-in `FBT_TEST_WIDGET=1` | exists; P0-13, P2-03 |
| `scripts/check-doc-links.sh` | markdown links into `docs/validation` | — | no (once in P0-01; by hand when receipts move) | P0-01 |
| `scripts/dagcheck.py` | PLAN.md §9.3 vs §9.4: undefined IDs, cycles, orphans, consumed-by, level table, weighted critical paths | python3 | no (review rounds, by hand) | P0-06 (the round-2 reviewer's script, `reviews/round-2-architecture.md` §D) |
| `scripts/mockwidth.py` | PLAN.md §5 and Appendix C mockups: cell widths, overflow against the declared 80/120/160, ragged boxes | python3 | no (review rounds, by hand) | P0-06 (round 2) |
| `scripts/lint-sizes.sh` | no file under `src/engine/` or `src/tui/` over 900 lines | — | yes | P1-02, P4-01 |
| `scripts/verify-determinism.sh` | `status`, `list`, `show`, `triage`, `config`, `capabilities`, `robot-docs --json` twice against a disposable daemon with `SOURCE_DATE_EPOCH` set, byte-identical | a build | yes (`native` job) | P1-11 |
| `scripts/gen-keymap.sh` | `robot-docs keys --json --surface all` → `plugin/oliver.torrents/keymap.json` and the README keys block | a build | yes (`keymap_json_matches`) | P4-02 |
| `scripts/testlog-summary.py` | the end-of-run table; exits 1 on any `ok: false` | python3 | yes | P0-15, P0-06 |
| `regression_tests/fbtlog_test.py` | the logging contract itself: a forced failure writes the expected lines and the summary counts it | python3 | yes | P0-15 |
| `regression_tests/fixtures.py` | fixture payloads, servers and `PATH` shims shared by `daemon.py`, `native.py`, `tui_smoke.py` | python3 | yes | P1-07 |
| `plugin/tests/qs-log.py` | runs `qs -p <dir>` and turns `FBTQS-*` lines into the JSONL contract | qs | opt-in | P0-15 |

### E.3 Fixtures

| Fixture | Used by |
|---|---|
| Local magnet with a 256 KiB payload and a local HTTP tracker (exists in `native.py`; `fixtures.magnet_fixture()` from P1-07) | every daemon case |
| Local seed/leech pair with `--throttle` (a second daemon in another temp dir) | P5-01, P5-04, P5-05, P6-01, P6-05, P7-06 |
| 64 MiB `.mkv`-named fixture (random bytes with a minimal Matroska header for the duration read) and a `.iso` fixture | P5-05 |
| Two-file torrent whose second file starts mid-piece | P5-05 (R15) |
| Corrupted-byte payload | P5-03 |
| Fake providers (`regression_tests/fixtures/providers/{alpha,beta}.py`: nova-style scripts printing fixed results with a duplicate pair and a mojibake name) | P3-01 |
| Local RSS feed, oversized feed | P3-02 |
| EZTV JSON fixture server (`fixtures.eztv_server()` over `regression_tests/fixtures/eztv/*.json`; the daemon is pointed at it with `FBT_EZTV_BASE_URL`) | P3-03, P3-13, P6-10, P3-18 |
| IMDb suggestion and poster fixture server (`fixtures.imdb_server()` over `regression_tests/fixtures/imdb/`); oversized poster | P3-04, P3-17 |
| Torznab fixture server with redirect cases (`fixtures.torznab_server()`) | P7-01 |
| `PATH` shims (`fixtures.path_shim(name, script)`): `notify-send`, `xdg-open`, `wl-paste`, `wl-copy`, `systemctl`, `systemd-inhibit`, `omarchy-launch-or-focus-tui`, each appending its argv to `<tmp>/shims/<name>.log` and optionally scripting stdout | P0-07, P1-04, P1-13, P6-09, P6-13 |
| `contracts/ipc/v1/*.ndjson` | Rust and QML contract tests |
| Theme TOML files under `/usr/share/omarchy/themes/`, copied into `contracts/ui/themes/*.toml` with `expected.json` by `contrast.py` (P2-08) | `contrast.py`, `qs-format.sh::dim_ladder`, `palette::tests` |
| Fixture states for `daemon.py --serve-fixture <state>` (`regression_tests/fixtures/states/<state>.json`, one per row of `plugin/tests/VISUAL.md`, plus the load fixtures `table-200.json` (P3-09) and `pieces-1000.json` (P6-02)) | `qs-visual.sh`, `qs-dashboard.sh` |
| `contracts/ui/states.json`, `contracts/ui/format.json` | `qs-format.sh`, `format::tests`, `qs-glyphs.sh` |

---

## Appendix F. CLI to API mapping (contract 1.2.0)

The CLI's JSON shapes and exit codes do not change for existing commands, with one declared
exception: `status` while stopped answers `ok: true, running: false` instead of `ok: false`
(DL-22), because the old shape was an error envelope without `code`/`code_name`
(`src/main.rs:410`) that no agent following `robot-docs` could parse as success. `capabilities
--json` 1.2.0 lists it under `changes: [{"since": "1.2.0", "breaking": true, "field":
"status.ok", …}]`, keeps `backend: "stopped"` and `app_open: false` in the stopped shape so
consumers of those fields keep working, and `robot-docs` says so (§10.3 Q12 asks whether to keep
`ok: false` for one more minor instead). The column "JSON shape" says whether the output is
unchanged, extended (new optional fields only), changed or new. The column "Starts" is §3.2.3's
rule.

| Command | API operations | Starts | JSON shape | Exit codes raised (A.3) |
|---|---|---|---|---|
| `triage` | `hello`, `torrents.list`, `transfer.info`, `app.network`, in-process shows/rss/search reads when down | no | extended: `backend.running`, `backend.daemon` (state, idle_exit_in_s, busy_reasons), `counts.ready`, `next_commands` gains `events --follow` | 0 |
| `capabilities`, `robot-docs [guide\|keys]` | none | no | extended (`contract_version` 1.2.0, new commands, `api`/`events`) | 0, 2 |
| `status [--quiet]` | `hello`, `transfer.info`, one `subscribe summary` (first `state` frame) | no | **changed** (DL-22): `ok: true, running: false, app_open: false, backend: "stopped", engine, total` when stopped; `running: true, connection: "api", daemon: {state, idle_exit_in_s?, busy_reasons, upgrade_pending}` when running; `history`, `all_time` added later (P6-03, P6-04) | 0; `--quiet`: 0 running, 4 stopped |
| `list [--active] [--state s] [--origin o]` | `torrents.list` | no (ledger fallback with a top-level `running: false`) | extended: the qBittorrent-shaped torrent stays (`state` keeps its qBittorrent string); new fields `native_state` (the §3.5 enum), `reason`, `checked`, `origin`, later `playable` (P1-11, P6-08, P5-05) | 0 |
| `show <sel>` | `torrents.get`, `torrents.files` | no | extended (`seeds`, `peers`, `playable`, `undo_until`) | 0, 6, 7 |
| `add <src> [--stopped] [--seed] [--save] [--category] [--quiet] [--wait] [--timeout] [--origin manual\|handler\|agent] [--notify]` | `torrents.add {…, origin}` (`--origin handler` is what the desktop entry passes; agents pass `--origin agent`; default `manual`), then `wait` | yes | unchanged (`{ok, action: "add", hash, name, source, sequential, first_last}`) plus `duplicate`, `deferred` (P7-09) | 0, 1, 2, 4, 5, 8 |
| `wait <sel>… [--timeout] [--interval] [--until complete\|playable\|stall-free]` | `subscribe torrents` (falls back to `torrents.get` polling at `--interval`); reconnects once across an exit-75 restart; exits 8 when the daemon idles out with the condition unmet | yes | extended (`playable`, `complete`, `stall_free`) | 0, 4, 6, 7, 8 |
| `start <sel>… [--force]` | `torrents.start` | yes | unchanged plus `skipped` | 0, 1 (`invalid_state`), 4, 6, 7 |
| `stop <sel>…` | `torrents.stop`; offline ledger write when down | no | unchanged | 0, 6, 7 |
| `delete <sel>… [--files --yes] [--dry-run]` | `torrents.remove` (`delete_files` from `--files`); offline ledger write when down and `--files` absent | `--files`: yes; else no | unchanged plus `undo_until` | 0, 3, 4, 6, 7 |
| `undo [<hash>]` (new, P6-08) | `torrents.add {source: {restore: hash}}` for the hash, or for every hash of the last keep-files batch when omitted | yes | new `{ok, action: "undo", hashes}` | 0, 1 (`invalid_state` after the window), 4, 6 |
| `seed`, `noseed <sel>…` | `torrents.set_seed` | yes | unchanged | 0, 4, 6, 7 |
| `seq`, `first-last` | none (stubs; `src/main.rs:314-318`) | no | unchanged refusal envelope | 2 (`unsupported`) |
| `reconcile` | none (the P0-07 stub: `{ok: true, touched: 0, note: "native scheduling is intrinsic"}`) | no | unchanged | 0 |
| `open <sel>` | `torrents.open_target` then `xdg-open` | yes | unchanged | 0, 4, 6, 7 |
| `recheck <sel>… [--wait] [--timeout]` (new, P5-03) | `torrents.recheck`, then `mutation.result` with `--wait`; `all` is sent in chunks of 256 | yes | new (`{ok, action: "recheck", count, hashes, pending: true}`; with `--wait` also `rechecked`, `failed`) | 0, 1 (staged or no metadata), 2, 4, 6, 7, 8 (`--wait --timeout`) |
| `reannounce <sel>…` (new, P6-01) | `torrents.reannounce` | yes | new (`{ok, action: "reannounce", count, hashes}`) | 0, 1 (not live), 4, 6, 7 |
| `queue <sel> top\|up\|down\|bottom` (new, P6-05) | `torrents.queue_move` (exactly one torrent; `all` is refused) | yes | new (`{ok, action: "queue", hash, queue_pos}`) | 0, 1 (no queue position), 2 (`all`), 4, 6, 7 |
| `limits [--down] [--up] [--alt on\|off] [--alt-down] [--alt-up]` (new) | `transfer.set_limits`, `transfer.set_alt`; `app.settings.set` when down | only to apply live | new | 0, 2 |
| `limit <sel> --down N` (new, P7-06) | `torrents.set_download_limit` | yes | new | 0, 2, 4, 6, 7 |
| `move <sel> <path>` (new, P7-02) | `torrents.move` | yes | new | 0, 2, 4, 6, 7, 8 |
| `rename <sel> <name>` (new, P7-03) | `torrents.rename` | yes | new | 0, 2, 4, 6, 7 |
| `add-clipboard` | `wl-paste` then `torrents.add` (`origin: "clipboard"`) | yes | unchanged | 0, 2, 4 |
| `rss list\|add\|remove\|refresh\|items` | `rss.*` or in-process | no | unchanged | 0, 2, 5, 6 |
| `rss grab` | `rss.grab` | yes | unchanged | 0, 4, 5, 6 |
| `shows list\|follow\|unfollow\|check\|episodes\|seen` | `shows.*` or in-process | no | unchanged | 0, 2, 5, 6, 7 |
| `shows grab` | `shows.grab` | yes | unchanged | 0, 4, 5, 6 |
| `shows quality <show> <q>` (new) | `shows.set_quality` or in-process | no | new | 0, 2, 6, 7 |
| `search <q> [--wait] [--limit] [--plugins] [--install] [--install-url]` | `search.*` or in-process | no | unchanged (plus `providers[]` per result) | 0, 2, 5 |
| `imdb <name> [--refresh]` | `imdb.lookup` or in-process | no | unchanged | 0, 5, 6 |
| `up [--ui]` | `ensure_daemon` (`systemctl --user start`); `--ui` summons the dashboard or runs the TUI | yes | unchanged | 0, 4 |
| `down` | `app.shutdown {reason: "user"}` (the owner's stop, A1; no prompt: typing the command is the deliberate act, and agents call it) | no | unchanged | 0, 4 (`--no-start` only), 8 |
| `restart` (new) | `app.restart {scope: daemon}` | no | new | 0, 4, 8 |
| `dashboard [--view v] [--query q]` (new) | `ensure_daemon`, then `omarchy-shell shell summon oliver.torrents '<payload>'`; falls back to `tui` when the shell or plugin is absent | yes | new | 0, 4, 5 |
| `tui [--search\|--rss\|--shows]` | `ensure_daemon`, presence `hello`, then data (compat until P4-03, API after) | yes | n/a | 0, 2 (no TTY), 4 |
| `log [--follow] [--level] [--source] [--json]` (new) | `app.log`, topic `log` | no | new (`entries: [LogEntry]`; `--follow` prints NDJSON) | 0, 4 |
| `events [--follow] [--since-seq] [--kinds] --json` (new) | `subscribe events {since_seq}` (replay, then live) | no | new (NDJSON, one event per line, each with `seq` and `instance`) | 0, 2 (bad kind), 4 |
| `api <op> [--arg k=v…] [--json-args] [--ref]` (new) | the named operation | per op (`mutating` column of A.4..A.6) | new (the response envelope verbatim under `result`) | every code of A.3 |
| `settings [--max-downloads] [--encryption] [--vpn] [--interfaces] [--idle-secs] [--handler] [--clipboard-watch] [--port]` | `app.settings.set` when running; file write when down | only to apply live | extended | 0, 2 |
| `config` | none | no | unchanged plus the new keys | 0 |
| `setup [--magnet] [--save] [--plugin=link\|copy] [--dry-run] [--uninstall] [--purge]` | none (files, systemd, shell IPC) | no | extended (`actions[]`) | 0, 1, 3 (`--purge` without `--yes`) |
| `register-magnet` | none (`xdg-mime default`, desktop entries; the same step as `setup --magnet`) | no | unchanged | 0, 1 |
| `serve [--idle-exit-secs] [--no-notify]` | is the daemon | n/a | n/a | 0, 1 (`already_running`), 70 (panic), 75 (binary replaced while idle); `capabilities.exit_codes` notes that these two belong to the unit, not to the 0..8 dictionary |

Reachability of every CLI exit code: 1 (`start` on `done`; `internal`), 2 (`seq`; `api nope`;
bad flag), 3 (`delete --files` without `--yes`), 4 (`--no-start` with a stopped daemon; `events`
when down; `status --quiet` when stopped; `busy`), 5 (`search` provider failure), 6 (`show zzzz`),
7 (an ambiguous selector), 8 (`wait --timeout`). `agent_contract.py::exit_code_dictionary_total`
exercises each one, and `agent_contract.py::retryable_never_maps_to_1` asserts that no API error
with `retryable: true` maps to exit 1.

Determinism rule for every `--json` output: keys in a fixed order (serde struct order), arrays in
a documented order (torrents by `added_at` descending, files by index, releases newest first then
`key`, search results by seeders descending then name then first provider, feeds by `id`, feed
items by `published_at` descending then `id`, shows by title, trackers by `url`),
no wall-clock fields except documented `*_at` values, which `SOURCE_DATE_EPOCH` does not touch but
which the determinism test masks; `NO_COLOR`, `CI` and a non-TTY stdout never change the bytes.
