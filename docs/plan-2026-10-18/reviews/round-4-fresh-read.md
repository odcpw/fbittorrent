# Round 4: fresh read of `PLAN.md` (6,688 lines, round 3)

Reviewer: independent, cold read, 2026-10-18. Read in full: `PLAN.md`, `review-log.md`,
`README.md`, `reality-check.md`, the cited parts of `frontend-options.md` and
`omatorrent-research.md`, the `evidence/` files, `screens/tui-real.png` and
`screens/30-bar-right-widgets.png`; checked against `src/`, `plugin/`, `vendor/`, `Cargo.*`,
`/usr/share/omarchy/shell/`, the ftui 0.6 registry crates, the installed Quickshell qmltypes,
`systemctl --user show`, `notify-send --help` and the 23 installed theme files. Nothing under the
repository was modified; no git state command was run. Scratch files live under the session
scratchpad (`r4/`). Every removed line of every hunk below was checked verbatim against
`PLAN.md` by script (Section H).

Skills applied: `planning-workflow` (review bar, validation loop), `ux-audit`,
`agent-ergonomics-and-intuitiveness-maximization-for-cli-tools`,
`just-say-no-to-process-porn-and-ceremony`.

## Verdict

**Close, but not ready to convert to beads as it stands.** The architecture, the API, the
design language and the item structure are settled and well grounded (Section F: 30 of 30
re-checked citations land on the quoted lines except one path slip; all 8 numeric claims
reproduce). What is still wrong is concentrated in two places: (1) the **M1a chain**, the first
thing the owner will live with, has three gaps that would make the first demo fail or regress the
bar (the demo installs with a script that does not exist until P2-07; the v0.1 popup, which must
survive until P2-03, decides "online" from `status.ok` and so stays visible forever once DL-22
lands; the §9.3 preamble says an operation during `Starting` is refused while §3.2.4 says it
waits, and a magnet click right after `READY=1` depends on the second); (2) a handful of
**protocol-level holes** the fresh agents hit at once (no shape for an error with no request;
`since_seq` missing from `subscribe`; completion does not fire the notify the bus waits on;
`subs: 8` is below the plugin's own topic count; the dashboard's bulk `X` cannot reach the
dashboard through `PanelKeyCatcher`). None needs a new round; all are integrator-sized. Apply
Section A and the Section E hunks, take B1–B3 (B4 after Q1), then convert. The plan should be
treated as at steady state after that: this round's structural findings are gaps, not
redesigns.

**Fix first:** A2, A3, A4, A5 (the M1a chain), then A6, A7, A13, A14 (protocol holes), then the
rest.

**Top 5 risks after this round**

1. **Q3 blocks the landing.** P0-04 waits on P0-02, which waits on the owner creating a public
   repository and answering a licence question the plan rightly refuses to answer for them. The
   rider's "make available to" clause plausibly covers this whole plan's execution model (agents
   read `vendor/asupersync/`), not just the fork; G1 says so plainly. Practically the public-fork
   path is sound (A24 adds the one missing ordering edge, B3 removes the runner that would break
   the moment the tree is archived).
2. **M1a is installed on the owner's machine for weeks with the old popup.** A3/A4/A5 make that
   period safe; without them the bar icon never hides and the "Added" toast is either doubled or
   actionless.
3. **The event bus's wake-up contract is subtly wrong** (A14): completion is signalled by
   `finished_notify`, not `state_change_notify`, and both are `notify_waiters` notifies, so the
   bus must register before it samples. P1-08's own fork test would fail as written.
4. **The 1 MiB line cap has no rule for growable results** (A16). A library of a few hundred
   torrents makes `torrents.list` violate the daemon's own cap.
5. **Look-and-feel.** The popup as specified is one row away from first-party (the key-hint row,
   C1); the dashboard's TUI-numbered tabs and bottom hint row read as an app, which is fine for a
   window; the TUI at 80 columns on the monochrome theme reads well, except the tab bar's new
   daemon state text does not fit next to the rates (A20).

---

## A. Defects

Each entry: location, what is wrong, evidence, hunk. Hunks are unified-diff hunks against
`docs/plan-2026-10-18/PLAN.md`; `-` lines are quoted verbatim from the round-3 text.

### A1. §9.2 says M1a is 26 S-units; §9.4 computes 31

**Where:** §9.2 line 2944. **Evidence:** §9.4's generated output: "M1a: 16 nodes (14 items),
weight 31". Recomputed by hand from the item sizes along the path
(1+2+1+2 + 2+2+2+2+4+4 + 2+1+4+2 = 31); the two post-P1-08 branches (daemon: 2+1+4+2, plugin:
2+2+4+1) both weigh 9, which is why M1 and M1a tie.

```diff
@@ §9.2 @@
-needs the pushed path. M1a, the end of P1, is the first installable build (26 S-units).
+needs the pushed path. M1a, the end of P1, is the first installable build (31 S-units, the same
+weight as M1: after P1-08 the daemon branch P1-12 → P1-17 → P1-11 → P1-15 and the plugin
+branch P2-01 → P2-02 → P2-03 → P2-05 both weigh 9, §9.4).
```

### A2. The §9.3 preamble contradicts §3.2.4 on `Starting`, and P0-10/A.3 on `start`

**Where:** §9.3 preamble, lines 2968–2975. **What is wrong:** (a) "any operation that needs the
engine answers `engine_unavailable` while the slot is `Parked` or `Starting`" versus §3.2.4:
"Operations that need the engine therefore wait only for the session (seconds, not minutes);
`torrents.add` keeps a 60 s bound and fails with retryable `engine_unavailable` past it". The
magnet-click path (Section D step 6) hits exactly this window: `systemctl start` returns at
`READY=1`, which §3.2.4 sends while the slot is still `Starting`. Under the preamble's wording
the first `add` after every idle exit would fail. (b) "`torrents.start` … work on every record"
versus P0-10 and A.3 (`start` on a completed stop-on-complete record is `invalid_state`). (c)
`set_files` on a dormant `done` torrent (include a skipped file later) is `invalid_state` under
the preamble and nowhere restores the record, while P1-16 restores for `start`, `set_seed`,
`recheck` and `open_target`.

```diff
@@ §9.3 preamble @@
-qBittorrent-shaped torrent of contract 1.x and gain only fields with new names (P1-11). **Defaults for edge states:** `torrents.start`, `stop`, `set_seed`, `remove` and `add` are ledger
-intents and work on every record (a `metadata` torrent can be paused, which stops the
-resolution); a mutation that needs a live engine handle (`recheck`, `reannounce`, `move`,
+qBittorrent-shaped torrent of contract 1.x and gain only fields with new names (P1-11). **Defaults for edge states:** `torrents.start`, `stop`, `set_seed`, `remove` and `add` are ledger
+intents and work on every record (a `metadata` torrent can be paused, which stops the
+resolution), with one carve-out: `start` on a completed stop-on-complete record is
+`invalid_state` with the hint `fbittorrent seed <hash>` (P0-10, A.3) and `start all` lists it
+under `skipped`; a mutation that needs a live engine handle (`recheck`, `reannounce`, `move`,
 `set_files`, `set_upload_limit`, `queue_move`, `force_start`) on a torrent that has none (dormant,
 staged, `metadata`, `done` without seeding) answers `invalid_state` unless the item says what it
-does instead (`recheck` restores a dormant torrent first); any operation that needs the engine
-answers `engine_unavailable` while the slot is `Parked` or `Starting` (§3.2.9); a read on such a
-torrent answers what the ledger knows and omits the rest (§3.4.9).
+does instead (`recheck`, and `set_files` with a non-empty `include`, restore a dormant torrent
+first, P1-16); any operation that needs the engine answers `engine_unavailable` while the slot
+is `Parked` (§3.2.9) and, while it is `Starting`, waits for the librqbit session for up to 60 s
+before answering retryable `engine_unavailable` (§3.2.4: a magnet click right after `READY=1`
+must not fail); a read on such a torrent answers what the ledger knows and omits the rest
+(§3.4.9).
```

```diff
@@ P1-16 Do @@
-  librqbit session. `torrents.start`, `torrents.set_seed {on: true}`, `torrents.recheck` and
-  `torrents.open_target` on a dormant record trigger the real restore with a check first; a
+  librqbit session. `torrents.start`, `torrents.set_seed {on: true}`, `torrents.recheck`,
+  `torrents.set_files` with a non-empty `include` and `torrents.open_target` on a dormant record
+  trigger the real restore with a check first; a
```

### A3. The M1a demo installs with `scripts/install.sh`, which P2-07 creates after M1a

**Where:** §9.7 M1a demo step (1) (line 5721); §8.6; P2-07; P1-04. **Evidence:** §9.4:
`P2-07 | P1-04, P1-11 | M1`; M1a's dependency list (line 3917) does not include P2-07. At M1a
the owner must replace a running binary (`cp` fails with `ETXTBSY`, which M1 demo step 9
already notes) and run `setup` so the unit, the `--origin handler` desktop entry and
`daemon-reload` exist; nothing in P1 says so. The script is ten lines; it belongs with the unit.

```diff
@@ P1-04 Do @@
-  add %u`) no longer opens a terminal; `fbittorrent.desktop` runs `fbittorrent dashboard`. `FBITTORRENT_UNIT` (default `fbittorrent.service`) names the unit
+  add %u`) no longer opens a terminal; `fbittorrent.desktop` runs `fbittorrent dashboard`. Create
+  `scripts/install.sh` here (§8.6; P2-07 only extends it): `cargo build --release --locked`, copy
+  the binary to `~/.local/bin/fbittorrent.tmp`, `mv` it over `~/.local/bin/fbittorrent` (a plain
+  `cp` onto a running binary fails with `ETXTBSY`), then `fbittorrent setup --magnet`, so the
+  M1a build is installed the same way as every later one and `setup` has run before the first
+  magnet click. `FBITTORRENT_UNIT` (default `fbittorrent.service`) names the unit
```

```diff
@@ P2-07 Do @@
-- *Do:* §8.6: `scripts/install.sh` (`cargo build --release --locked`; copy the binary to
-  `~/.local/bin/fbittorrent.tmp` and `mv` it over `~/.local/bin/fbittorrent`; then `fbittorrent
-  setup --magnet --plugin=link`); `setup` flags `--magnet`, `--save`, `--plugin=link|copy`,
+- *Do:* §8.6: `scripts/install.sh` (created by P1-04) gains `--plugin=link` on its `setup` call
+  (`fbittorrent setup --magnet --plugin=link`); `setup` flags `--magnet`, `--save`, `--plugin=link|copy`,
```

```diff
@@ §8.6 @@
-- `scripts/install.sh` (P2-07): build with `--release --locked`; install the binary into
+- `scripts/install.sh` (P1-04; P2-07 adds `--plugin=link`): build with `--release --locked`; install the binary into
```

### A4. At M1a the v0.1 popup reads the new `status` shape and never hides

**Where:** §0.6 line 202, §9.7 line 5713, P0-07, P1-11. **Evidence:**
`plugin/oliver.torrents/Service.qml:15` is
`readonly property bool online: status.ok === true`; `Panel.qml:31` is `visible: svc.online`;
`Model.js:102` returns "Backend offline" only when `!s.ok`. DL-22 makes the stopped shape
`{ok: true, running: false, …}` at P1-11, and the old popup is kept until P2-03 ("the old popup
still works", §0.6, §9.7). With DL-22 in and this line unchanged, the icon is visible while the
daemon is stopped and the hero says "No torrents": the owner's "visible only while the app runs"
rule breaks for the whole M1a period. Also, the plan says the old popup works "through the
compat socket"; it never touches a socket, it spawns the CLI (`Service.qml:80-118`), and what
must stay compatible is the `status --json` *running* shape's 1.x fields, which P1-11 leaves as
"…". One-line fix in the item that already edits `Service.qml`.

```diff
@@ P0-07 Do @@
-  (`Service.qml`'s argv builder). Contract 1.1.1 (`src/agent.rs:134-210`) lists the flag under
-  `global_flags[]`.
+  (`Service.qml`'s argv builder). `Service.qml:15` becomes
+  `readonly property bool online: status.ok === true && status.running !== false`, so the widget
+  keeps hiding when `status` adopts the 1.2.0 stopped shape `{ok: true, running: false}` (DL-22)
+  at M1a, weeks before P2-03 replaces it. Contract 1.1.1 (`src/agent.rs:134-210`) lists the flag under
+  `global_flags[]`.
```

```diff
@@ P0-07 Accept @@
-  `reconcile` and `--no-start list` never invoke the launcher; a widget action while the app is
-  closed reports "fbittorrent is closed".
+  `reconcile` and `--no-start list` never invoke the launcher; a widget action while the app is
+  closed reports "fbittorrent is closed"; `Service.qml` tests `status.running !== false` in its
+  `online` expression (a one-line `rg` in `all.sh` until P2-04's lint takes it over).
```

```diff
@@ P1-11 Do, status @@
-  (§3.2.13): running → `{ok: true, running: true, connection: "api", backend: "native", app_open:
-  <interactive count > 0>, daemon: {state, idle_exit_in_s?, busy_reasons, upgrade_pending}, …}`
+  (§3.2.13): running → `{ok: true, running: true, connection: "api", backend: "native", app_open:
+  <interactive count > 0>, daemon: {state, idle_exit_in_s?, busy_reasons, upgrade_pending}}` plus
+  every 1.x running field unchanged (`engine`, `dl`, `up`, `active`, `queued`, `seeding`, `stopped`,
+  `total`, `torrents: [<cli::shapes::torrent()>]` with the active ones first then up to 12 others,
+  `src/main.rs:416-430`), because the v0.1 popup reads them until P2-03 (`Model.js:102-115`)
```

```diff
@@ §0.6 @@
-`api` serve agents, and the old popup still works through the compat socket (§9.7 "M1a demo
+`api` serve agents, and the old popup still works, now through the new CLI's `status --json` and
+`--no-start` actions (P0-07, P1-11; it never touches a socket itself) (§9.7 "M1a demo
```

```diff
@@ §9.7 milestone table @@
-| M1a | Magnet clicks without terminals; downloads survive closing the TUI; idle exit with `fbittorrent status` showing the countdown; completion and added notifications; `events`/`api` for agents; the old popup still works through the compat socket | v0.2.0-pre (installed) |
+| M1a | Magnet clicks without terminals; downloads survive closing the TUI; idle exit with `fbittorrent status` showing the countdown; completion and added notifications; `events`/`api` for agents; the old popup still works through the new CLI (`status --json`, `--no-start` actions) | v0.2.0-pre (installed) |
```

### A5. Who posts the "Added" notification is ambiguous, and the two readings both fail

**Where:** §3.8 notifier table, P1-13 Do. **Evidence:** §3.8's row says "added (handler; and the
CLI when stdout is a TTY or `--notify`…)", §6.3 says the daemon emits `torrent.added`, P1-13 says
the daemon's notifier posts "added" with `undo`/`window` **and** "the CLI's added notification
only on a TTY or with `--notify`". Read as "both post": a click yields two toasts. Read as "the
CLI posts": `undo` needs `notify-send -A`, which implies `--wait` (verified: `notify-send --help`),
so `fbittorrent add` would block for the toast's lifetime and the `undo` handler would die with
the handler process. The daemon must own the actioned toast; the CLI keeps its plain one.

```diff
@@ P1-13 Do @@
-  waiting / back" (no actions, once per transition); bodies escaped for `&` and `<`; `notify =
-  false` (config) and `serve --no-notify` disable everything; the CLI's added notification only on
-  a TTY or with `--notify`. The SVG icon `assets/fbittorrent.svg` (monochrome, the `󰇚` shape)
+  waiting / back" (no actions, once per transition); bodies escaped for `&` and `<`; `notify =
+  false` (config) and `serve --no-notify` disable everything. Who posts "Added": the daemon, with
+  its two actions, only when `torrents.add` carries `origin: "handler"` (the desktop entry; the
+  `undo` action must outlive the short-lived `add` process); the CLI keeps today's plain
+  `notify-send` (no actions, no `--wait`, `src/main.rs:1296-1300`) only on a TTY or with
+  `--notify`, and never for `--origin handler`, so a click produces exactly one toast. The SVG icon `assets/fbittorrent.svg` (monochrome, the `󰇚` shape)
```

### A6. `subs: 8` is below the plugin's and the TUI's own topic count

**Where:** §3.4.10, §3.4.3 example (line 1062), P1-06, A.6. **Evidence:** the plugin's one
connection (§4.4 rule 8) carries the bar, the popup and the dashboard. The dashboard's baseline
is `summary`, `torrents`, `network`, `settings`, `shows` and `rss.feeds` (the unread badges on
the tabs need the last two whenever the window is visible, §4.6), plus the view's topic
(`search:<id>`, `shows.releases:*`, `rss.items:<feed>`) plus up to two detail topics (Files and
the pane's Log, P3-10): nine. P4-03's TUI mirrors "the `summary`, `torrents`, `network`,
`settings`, `rss.feeds`, `shows` topics and the detail topics of the visible torrent": eight or
nine. The ninth `subscribe` is `busy`. The limit is a DoS bound, not a design number; 16 costs
nothing.

```diff
@@ §3.4.3 hello example @@
-     "limits":{"line_in":65536,"line_out":1048576,"inflight":8,"subs":8},
+     "limits":{"line_in":65536,"line_out":1048576,"inflight":8,"subs":16,"hashes":256},
```

```diff
@@ §3.4.10 @@
-| Subscriptions per connection | 8 | The dashboard needs about 5 at once |
+| Subscriptions per connection | 16 | The plugin's one connection carries the bar, the popup and the dashboard: `summary`, `torrents`, `network`, `settings`, `rss.feeds` and `shows` (badges) plus a view topic and up to two detail topics, nine on a bad day; the TUI mirrors the same six plus details (P4-03). 8 would refuse the ninth with `busy`; 16 costs nothing and is still a bound |
```

```diff
@@ P1-06 Do @@
-  now; later items append theirs) and `limits {line_in: 65536, line_out: 1048576, inflight: 8,
-  subs: 8, hashes: 256}`; line caps (64 KiB in, 1 MiB out; over → `invalid_message` and close);
+  now; later items append theirs) and `limits {line_in: 65536, line_out: 1048576, inflight: 8,
+  subs: 16, hashes: 256}`; line caps (64 KiB in, 1 MiB out; over → `invalid_message` and close);
```

```diff
@@ A.6 subscribe (also carries A12: since_seq) @@
-| `subscribe` | `topics: [str]` (1..8), `interval_ms: u64?` (250..10000; applies to every topic of the call; when omitted, each topic uses its A.7 default) | `{sub: u64, topics: [str]}` | no | P1-08 |
+| `subscribe` | `topics: [str]` (1..16, the per-connection bound of §3.4.10), `interval_ms: u64?` (250..10000; applies to every topic of the call and is ignored by the "on change" topics of A.7; when omitted, each topic uses its A.7 default), `since_seq: u64?` (only with the `events` topic, A.7) | `{sub: u64, topics: [str]}` | no | P1-08 |
```

### A7. The dashboard's bulk-remove key `X` cannot reach the dashboard

**Where:** Appendix B "Bulk remove the filtered set" row (line 6279), §4.12, P6-08.
**Evidence:** `Ui/PanelKeyCatcher.qml:93-95`:
`if (event.key === Qt.Key_Delete || event.text === "x" || event.text === "X") { deleteRequested(); …}`.
The catcher consumes `x`, `X` and Delete into one modifier-less `deleteRequested()` before any
item sees the key (`Keys.priority: Keys.BeforeItem`), so the dashboard cannot tell `X` (bulk,
keep files) from `x` (one torrent). A window-scoped `Shortcut` fires before key events reach the
focus item, which is the one way to keep the letter.

```diff
@@ Appendix B @@
-| Bulk remove the filtered set (new, P6-08) | `X` | `X` | n/a |
+| Bulk remove the filtered set (new, P6-08) | `X` | `X`, bound through a `Shortcut { sequence: "Shift+X"; context: Qt.WindowShortcut }` in `Dashboard.qml`, because `PanelKeyCatcher` folds `x`, `X` and Delete into one `deleteRequested()` with no modifier (`Ui/PanelKeyCatcher.qml:93-95`) | n/a |
```

```diff
@@ §4.12 @@
-- `PanelKeyCatcher` turns keys into signals: `j`/`k`/`h`/`l` and arrows move, Enter and Space
-  activate, `x` and Delete request deletion, Esc closes, Tab switches, Ctrl+j/k reorder when
-  `reorderable`, any other character arrives as `textKey(text, modifiers)`
-  (`Ui/PanelKeyCatcher.qml:36-99`). It takes keys before items, so text fields set `blocked`
+- `PanelKeyCatcher` turns keys into signals: `j`/`k`/`h`/`l` and arrows move, Enter and Space
+  activate, `x`, `X` and Delete request deletion (one `deleteRequested()`, no modifier, so the
+  dashboard's bulk `X` is a window `Shortcut` that fires before the catcher, Appendix B), Esc
+  closes, Tab and Shift+Tab emit `tabRequested(±1)`, Ctrl+j/k reorder when `reorderable`, any
+  other character arrives as `textKey(text, modifiers)`
+  (`Ui/PanelKeyCatcher.qml:36-99`). It takes keys before items, so text fields set `blocked`
```

### A8. §4.3.2 persists the sort key "in the plugin settings"; §4.6 says `dashboard.json`

**Where:** §4.3.2 line 1706–1707 versus §4.6 (details pane paragraph) and P3-10. **Evidence:**
round 3 decided the plugin-owned state file because panels receive no `settings`
(`shell.qml:1396-1406`; the only writer is `updateEntryInline`, which rewrites the bar entry) and
updated §4.6 and P3-10, not §4.3.2. The same bullet also leaves which headers are clickable
open (the P3-09 probe asked).

```diff
@@ §4.3.2 @@
-- Sorting: click a header or press `s`; the sorted header is in foreground with an arrow glyph
-  (omaqbt `TorrentTable.qml:69-72`); the sort key is persisted in the plugin settings.
+- Sorting: click any header (it sorts by that column, ↑, PEERS and RATIO included) or press `s`
+  (which cycles the §4.6 list); the sorted header is in foreground with an arrow glyph
+  (omaqbt `TorrentTable.qml:69-72`); the sort key and direction persist in the plugin-owned
+  `~/.local/state/fbittorrent/dashboard.json` (§4.6; panels receive no `settings`).
```

### A9. §7.3 and P7-01 disagree on Torznab's caps and on where the key lives

**Where:** §7.3 lines 2735–2739 versus §6.3 row "Torznab" and P7-01. **Evidence:** §7.3: "2 MiB
and 15 s", `[[torznab]] name, url, api_key` or `api_key_file`; P7-01 and §6.3: "30 s timeout and a
4 MiB response cap", the key "read from a 0600 file opened with `O_NOFOLLOW`". An inline
`api_key` in a TOML that `config` prints is the leak §3.9 forbids.

```diff
@@ §7.3 @@
-  Configuration in `~/.config/fbittorrent/indexers.toml` (0600, opened with `O_NOFOLLOW`,
-  refused if group- or world-readable): `[[torznab]] name, url, api_key` or `api_key_file`.
-  Rules: HTTPS is required when an API key is set, except on loopback; redirects to another origin
-  are refused; the key never appears in logs, errors or argv; responses are capped at 2 MiB and
-  15 s; XML parsing uses `quick-xml`, already a dependency (`Cargo.toml:34`). Why it is worth
+  Configuration in `~/.config/fbittorrent/indexers.toml` (0600, opened with `O_NOFOLLOW`,
+  refused if group- or world-readable): `[[torznab]] name, url, api_key_file` (the key lives only
+  in its own 0600 file, never inline, so `settings`/`config` output can never print it; P7-01).
+  Rules: HTTPS is required when an API key is set, except on loopback; redirects to another origin
+  are refused; the key never appears in logs, errors or argv; responses are capped at 4 MiB and
+  30 s (the numbers P7-01 and §6.3 implement); XML parsing uses `quick-xml`, already a dependency (`Cargo.toml:34`). Why it is worth
```

### A10. §5.6 opens the TUI details modal on Files; the mockup, Appendix B and the dashboard open on Overview

**Where:** §5.6 table, Torrents row (line 2448). **Evidence:** the details mockup three
paragraphs below shows `[Overview]  Files  Peers  Log`; Appendix B: "Enter opens details (Overview
first)"; §4.7 Overview is the first tab.

```diff
@@ §5.6 table @@
-| Torrents | State (9), Name, %, ↓ when live, ETA. No side pane; Enter opens details full-screen (Files first), `h`/Esc returns. | Adds Size and Peers, and a one-line strip above the footer with the selected torrent's state, reason and path. | Side pane with the IMDb card and details, as today, plus the State column. |
+| Torrents | State (9), Name, %, ↓ when live, ETA. No side pane; Enter opens details full-screen (Overview first, as the mockup below, Appendix B and the dashboard; `]` reaches Files), `h`/Esc returns. | Adds Size and Peers, and a one-line strip above the footer with the selected torrent's state, reason and path. | Side pane with the IMDb card and details, as today, plus the State column. |
```

### A11. §4.5's visibility expression hides the icon during the "Reconnecting" state §4.11 draws

**Where:** §4.5 line 1880, §4.11 rows "Connecting" and "Reconnecting". **Evidence:** a crash
closes the socket without `bye`; `online` turns false; `stoppedRecently` is set only by a `bye`.
`visible: svc.online || showWhenStopped || root.opened || svc.stoppedRecently` therefore hides the
icon instantly, systemd restarts the daemon 2 s later and the icon reappears: a flicker, and
§4.11's "last known, dim" icon is never shown.

```diff
@@ §4.5 Visibility @@
-Visibility: `visible: svc.online || showWhenStopped || root.opened || svc.stoppedRecently`,
+Visibility: `visible: svc.online || showWhenStopped || root.opened || svc.stoppedRecently || svc.reconnecting`
+(`reconnecting` is true from an EOF without `bye` until the backoff reaches its 10 s cap, about
+15 s, so a crash-restart under `Restart=on-failure` shows the §4.11 "Reconnecting" icon instead
+of a vanish-and-return),
```

### A12. `since_seq` is used by A.7 but is not an argument of `subscribe`; `oldest_seq` has no frame

**Where:** A.6 `subscribe`, A.7 events paragraph, A.8 `snapshot.begin`. **Evidence:** A.7 says
"Subscribing to `events` with `since_seq`" and "`details.oldest_seq` in the result", but the
`subscribe` result is `{sub, topics}` and A.6's args have no `since_seq`. The A.6 hunk under A6
adds the argument; `oldest_seq` goes on the frame that carries the `resync` reason.

```diff
@@ A.7 events @@
-a `since_seq` older than the ring answers `snapshot.begin {reason: "resync"}` with
-`details.oldest_seq` in the result so the client knows it missed events. `seq` restarts at 1 with
+a `since_seq` older than the ring answers `snapshot.begin {reason: "resync", oldest_seq}` (A.8;
+the `subscribe` result itself is unchanged) so the client knows it missed events. `seq` restarts at 1 with
```

```diff
@@ A.8 snapshot.begin @@
-| `snapshot.begin` | `sub: u64`, `topic: str`, `rev: u64`, `count: u64`, `reason: enum(subscribe\|engine_restart\|resync)` |
+| `snapshot.begin` | `sub: u64`, `topic: str`, `rev: u64`, `count: u64`, `reason: enum(subscribe\|engine_restart\|resync)`, `oldest_seq: u64?` (only with `reason: resync` on the `events` topic, A.7) |
```

### A13. An error with no request to answer has no frame shape

**Where:** A.1, A.8, §3.4.2. **Evidence:** A.1 defines responses as `{"id", "ok": false, …}`;
the 33rd connection's `busy`, a line over the cap, `hello_required`, `proto_unsupported` and a
reused or non-integer `id` all have no usable `id`. P1-06's `garbage_table` test needs a shape to
assert. (Found by the P1-06 probe.)

```diff
@@ A.1 @@
 - Events: `{"ev": str, …}`; subscription events carry `sub`, `topic` and `rev`.
+- An error with no request to answer (a line over the cap or not an object, `hello_required`,
+  `busy` for the 33rd connection, `proto_unsupported`) is sent as the event
+  `{"ev": "error", "error": Error}` and is always followed by `bye` and the close (A.8);
+  `invalid_args` for a bad `id` (0, negative, non-integer, reused) answers with `"id": null`
+  and keeps the connection.
```

```diff
@@ A.8 @@
 | `bye` | `reason: enum(shutdown\|idle\|restart\|upgrade\|slow_client\|hello_required\|invalid_message)` |
+| `error` | `error: Error` (A.3); only for errors that have no request to answer (A.1); always followed by `bye` |
```

### A14. The bus waits on the wrong notify for completion, and the ledger has no change counter

**Where:** §3.4.7, P1-08 Do. **Evidence (fork):** completion fires
`self.finished_notify.notify_waiters()` (`vendor/rqbit/librqbit/src/torrent_state/live/mod.rs:910`;
the field at `:201`), not `state_change_notify`; both are `Notify` with `notify_waiters`, which
wakes only futures already registered, so a bus that samples state and then registers misses the
transition; at `torrent_state/mod.rs:333` the notify precedes the `Error` assignment. P1-08's fork
unit test "wakes on … completion" would fail as written. **Evidence (ledger):** the plan's
"ledger's change counter" does not exist; `src/engine.rs:154` is an async `Mutex<Ledger>` and the
bus is a std thread (§3.10). (Found by the P1-08 probe; verified.)

```diff
@@ §3.4.7 @@
-  waits on each live torrent's `ManagedTorrent::state_changed()` (a fork accessor over the
-  existing `pub(crate) state_change_notify`, `vendor/rqbit/librqbit/src/torrent_state/mod.rs:212`,
-  notified on every transition at `:333,412-460,518-525`) and on the ledger's change counter, and
+  waits on each live torrent's `ManagedTorrent::state_changed()` (a fork accessor that selects over
+  the existing `pub(crate) state_change_notify`, `vendor/rqbit/librqbit/src/torrent_state/mod.rs:212`,
+  notified on every transition at `:333,412-460,518-525`, and over the live state's
+  `finished_notify`, `torrent_state/live/mod.rs:201,910`, which is the only signal completion
+  sends) and on the ledger's change counter, and
```

```diff
@@ P1-08 Do @@
-  structural frames driven by a fork accessor `ManagedTorrent::state_changed()` (added to
-  `vendor/rqbit/librqbit/src/torrent_state/mod.rs` over the existing `state_change_notify` at
-  `:212`; a fork unit test asserts it wakes on pause, unpause, completion and error) and by the
-  ledger's change counter; rate and progress changes coalesced to `interval_ms` (clamped
+  structural frames driven by a fork accessor `ManagedTorrent::state_changed(&self) -> impl Future`
+  (added to `vendor/rqbit/librqbit/src/torrent_state/mod.rs`; it selects over the existing
+  `state_change_notify` at `:212` **and** the live state's `finished_notify`
+  (`torrent_state/live/mod.rs:201,910`), because completion fires only the latter; both are
+  `Notify` with `notify_waiters`, so the bus registers `notified()` before it samples the state and
+  re-samples after every wake, which also covers `:333`, where the notify precedes the `Error`
+  assignment; a fork unit test asserts it wakes on pause, unpause, completion and error) and by the
+  ledger's change counter (P1-19 adds `Ledger.generation: AtomicU64`, bumped on every save, and a
+  `std::sync::Condvar` the bus thread waits on; the bus reads torrent stats through the engine's
+  `block_on` path like dispatch does, `src/engine.rs:577-583`, never by locking the async
+  `Mutex<Ledger>` of `src/engine.rs:154` from a std thread); rate and progress changes coalesced to `interval_ms` (clamped
```

### A15. "Rate and progress changes" is not a closed set

**Where:** §3.4.7. **Evidence:** the bus must decide per field whether a change rides the tick or
goes out at once; the plan names examples only. (P1-08 probe.)

```diff
@@ §3.4.7 @@
-- `interval_ms` is clamped to 250..10000 (default 1000). Structural changes (added, removed, state
-  change, completion, playable, error, network change) go out immediately; rate and progress
-  changes are coalesced to the interval. "Immediately" is event-driven, not tick-driven: the bus
+- `interval_ms` is clamped to 250..10000 (default 1000). Structural changes (added, removed, state
+  change, completion, playable, error, network change) go out immediately; rate and progress
+  changes are coalesced to the interval. The coalesced set is closed: `progress`,
+  `completed_bytes`, `uploaded_bytes`, `dl_bps`, `ul_bps`, `eta_s`, `peers`, `seeds`, `ratio`,
+  `playable.buffered_s`, `playable.finishes_in_s`, every `Summary` rate and count and every
+  `Peer`/`Tracker` counter; a change to any other field of any object is structural (so a
+  `playable.ready` or `stall_free` flip, a `name` change or a new `reason` go out at once).
+  "Immediately" is event-driven, not tick-driven: the bus
```

### A16. The 1 MiB server line cap has no rule for results that grow with the library

**Where:** §3.4.1, A.5. **Evidence:** `torrents.list` returns every torrent; a `Torrent` with a
long name is "under 2 KiB", so about 512 torrents exceed the daemon's own `line_out`, and the
client (QML) treats such a line as a protocol violation. The cap exists for the plugin's
`SplitParser` (§4.4 rule 5); the plugin never needs these two results (it subscribes), the CLI
and TUI read with `BufRead`. The honest rule is to say which frames the cap binds rather than to
add pagination for a library this owner does not have. (P1-06 probe.)

```diff
@@ §3.4.1 @@
-- One UTF-8 JSON object per line, terminated by `\n`. Client-to-server lines are at most 64 KiB;
-  server-to-client lines at most 1 MiB. A longer line gets `invalid_message` and the connection is
-  closed. A line that is not a JSON object (an array, a scalar, invalid UTF-8) gets the same.
+- One UTF-8 JSON object per line, terminated by `\n`. Client-to-server lines are at most 64 KiB;
+  server-to-client lines at most 1 MiB. A longer line gets `invalid_message` and the connection is
+  closed. A line that is not a JSON object (an array, a scalar, invalid UTF-8) gets the same. The
+  1 MiB bound is what the plugin's `SplitParser` relies on (§4.4 rule 5), so it is a hard bound on
+  every pushed frame (snapshots are chunked, §3.4.7) and on every result the plugin asks for; the
+  two results that grow with the library, `torrents.list` and `torrents.files`, are not capped
+  (the CLI and TUI read them with `BufRead`) and the plugin never calls them (it subscribes;
+  `plugin/tests/lint.sh` rejects the two op names in `plugin/`, P2-05).
```

### A17. §3.2.12's "replaced by `cp`" cannot happen

**Where:** §3.2.12 line 938. **Evidence:** M1 demo step 9 itself: "a plain `cp` fails with 'Text
file busy' while the daemon runs". The mechanism (inode change through `mv`) is right; the word
is wrong.

```diff
@@ §3.2.12 @@
-- Why: during development `~/.local/bin/fbittorrent` is replaced by `cp` while a long-lived daemon
+- Why: during development `~/.local/bin/fbittorrent` is replaced (by `install.sh`'s `mv`; a `cp`
+  onto a running binary fails with `ETXTBSY`) while a long-lived daemon
```

### A18. `agents:972` is not under `plugins/panels/`

**Where:** §4.2.2 line 1567. **Evidence:** `/usr/share/omarchy/shell/plugins/panels/` holds
`audio bluetooth clock disk-speedtest dropbox elsewhen monitor network power speedtest tailscale
weather wifiqr`; the agents panel is `/usr/share/omarchy/shell/plugins/agents/Panel.qml`, whose
line 972 is indeed `contentWidth: panel.fittedContentWidth(Style.space(380))`.

```diff
@@ §4.2.2 @@
-  `panels/network/Panel.qml:1109`, `bluetooth:670`, `tailscale:421`, `agents:972`; today's 400,
+  `panels/network/Panel.qml:1109`, `panels/bluetooth:670`, `panels/tailscale:421`,
+  `plugins/agents/Panel.qml:972`; today's 400,
```

### A19. The `dashboard` client kind is never sent and `agent` is never defined

**Where:** §3.4.3 (line 1066), A.2, A.4 `app.info`. **Evidence:** §4.4 rule 8: the plugin's one
connection says `kind: "bar"` and toggles `interactive` for the dashboard; no code path sends
`dashboard`, so `clients.dashboard` is a counter that stays 0 forever. `agent` has no sender
either; giving it one (the CLI under `--json`) makes the per-kind counters answer the question
P7-07's retirement review was kept for.

```diff
@@ §3.4.3 @@
-`client.kind` is one of `cli`, `tui`, `dashboard`, `bar`, `agent`, `test`. `interactive` can change
+`client.kind` is one of `cli` (a human at the CLI), `agent` (the CLI with `--json`, so the
+counters below tell agent use from human use), `tui`, `bar` (the plugin's one connection, which
+also serves the dashboard, §4.4 rule 8; there is no `dashboard` kind) and `test`. `interactive` can change
```

```diff
@@ A.2 @@
-| args | `proto: u64` (required; 1), `client: {name: str (≤ 64), version: str, kind: enum(cli\|tui\|dashboard\|bar\|agent\|test)}` (required), `interactive: bool` (default false) |
+| args | `proto: u64` (required; 1), `client: {name: str (≤ 64), version: str, kind: enum(cli\|agent\|tui\|bar\|test)}` (required; §3.4.3), `interactive: bool` (default false) |
```

```diff
@@ A.4 app.info @@
-| `app.info` | none | hello's `server`, `engine`, `instance`, `engine_epoch`, `state` plus `uptime_s: u64`, `upgrade_pending: bool`, `clients: {cli: u64, tui: u64, dashboard: u64, bar: u64, agent: u64, test: u64}` (connections since start), `subscriptions: {total: u64, by_topic: {str: u64}}` (live subscriptions, P1-08) | no | P1-06, P1-08 |
+| `app.info` | none | hello's `server`, `engine`, `instance`, `engine_epoch`, `state` plus `uptime_s: u64`, `upgrade_pending: bool`, `clients: {cli: u64, agent: u64, tui: u64, bar: u64, test: u64}` (connections since start), `subscriptions: {total: u64, by_topic: {str: u64}}` (live subscriptions, P1-08) | no | P1-06, P1-08 |
```

### A20. The TUI tab bar's daemon-state text does not fit at 80 columns

**Where:** §5.7 row 17, P4-07. **Evidence:** the §5.6 80×24 mockup's tab row is exactly 80 cells
with 14 cells of slack before `↓ 3.1 MB/s ↑ 32 KB/s`; D17's theme name freed 8 cells; "idle ·
stops in 4 min" is 21 cells. The text can only take the rates' place, which is harmless because
the state word matters only when nothing transfers.

```diff
@@ P4-07 Do @@
-  name in the query box), 17 (the tab bar shows the daemon state from `Summary.daemon`: "idle ·
-  stops in 4 min", "waiting for VPN"; the theme name is dropped) and the lower-severity notes of
+  name in the query box), 17 (the tab bar shows the daemon state from `Summary.daemon`: "idle ·
+  stops in 4 min", "waiting for VPN"; the theme name is dropped; the state text takes the place of
+  the rate text, which it replaces only while both rates are 0 or the engine is parked, because at
+  80 columns the tab row has 14 spare cells, not the 21 the text needs, and a transferring daemon
+  needs no state word) and the lower-severity notes of
```

### A21. "Each pane has a title row of `Style.space(30)`" and the §4.6 mockup disagree

**Where:** §4.2.2 lines 1575–1576. **Evidence:** the mockup's sidebar starts with "All 12" and
its list with the column header; no pane shows a caption, yet the chrome arithmetic (636/246/16)
counts a 30 px title per pane. Say which row is the title row so the arithmetic and the mockup
mean the same thing. (P3-09 probe.)

```diff
@@ §4.2.2 @@
-  `BorderSurface` with `Border.controlSpec("normal")`, padding `Style.spacing.lg`). Each pane has
-  a title row of `Style.space(30)` and a body. Chrome with both side panes docked is therefore
+  `BorderSurface` with `Border.controlSpec("normal")`, padding `Style.spacing.lg`). Each pane has
+  a title row of `Style.space(30)` and a body: the list's title row is its column header, the
+  details pane's is the torrent name with the tab strip, and the sidebar's is its "All" entry (no
+  separate caption, as the §4.6 mockup shows). Chrome with both side panes docked is therefore
```

### A22. The width tiers mix pixels and `Style.space` units

**Where:** §4.3.3. **Evidence:** `Commons/Style.qml:213`:
`effectiveSpacingScale: spacingScale * (spacingScaleWithFont ? fontScale : 1)`; `Style.space(210)`
is 210 px only at scale 1, while `FloatingWindow.width` is pixels. On a scaled shell the tier
boundaries (1600/1400/1100/900) would no longer match the pane arithmetic. (P3-09 probe.)

```diff
@@ §4.3.3 @@
-for it, not degraded to it). Each tier below is checked: list width = window − chrome (§4.2.2),
-NAME = list − fixed columns (§4.3.2); every tier keeps NAME ≥ 210 at its narrowest width, and
-column visibility is monotonic (a column hidden at one width is hidden at every narrower width).
+for it, not degraded to it). Each tier below is checked: list width = window − chrome (§4.2.2),
+NAME = list − fixed columns (§4.3.2); every tier keeps NAME ≥ 210 at its narrowest width, and
+column visibility is monotonic (a column hidden at one width is hidden at every narrower width).
+Every number here is in `Style.space` units: the tier is chosen from
+`width / Style.effectiveSpacingScale` (`Commons/Style.qml:213`, the font scale times the spacing
+scale), so a scaled shell keeps the same proportions; `qs-dashboard.sh::width_tiers` runs at
+scale 1 and logs the scale it saw.
```

### A23. P0-02 can run before P0-01 and remove the tree the worktree backup is meant to hold

**Where:** P0-02 Depends. **Evidence:** both items have "Depends on: none"; P0-02 step 6 removes
`vendor/asupersync/` after its own archive, and §8.1 step 1 (the whole-tree backup) is P0-01. The
item's "if P0-01 ran first, restore that one file from the archive" clause exists only because the
order is undefined; with B3 (retire the runner) that clause disappears and the edge is clean.

```diff
@@ P0-02 Depends @@
-- *Depends on:* none; blocked on the owner's answer to Q3 (the only owner action before P0-04),
-  because the answer decides whether `vendor/asupersync/` enters history in C3, and the owner
-  creates the repository.
+- *Depends on:* P0-01 (the worktree backup must exist before step 6 removes `vendor/asupersync/`);
+  blocked on the owner's answer to Q3 (the only owner action before P0-04),
+  because the answer decides whether `vendor/asupersync/` enters history in C3, and the owner
+  creates the repository.
```

(§9.4's rows for P0-01 and P0-02 follow from `regen94.py`.)

### A24. The §9.4 "critical path as the script prints it" is not the path the script printed

**Where:** §9.4 diagram (line 5642) and the weighted paths (5649–5650). **Evidence:**
`review-log.md`'s recorded output: `M1 <- M1a <- … <- M0 <- P0-09 <- P0-15 <- P0-04 <- P0-03`;
the plan prints `… ← M0 ← P0-06 ← P0-05 ← P0-04 ← P0-01`. Both are longest paths (the P0
prefixes tie at 6: 1+2+1+2 and 1+2+1+2), so nothing is wrong in substance; the claim "as the script
prints it" is. No hunk: the integrator regenerates §9.4 anyway after A23; the regenerated text
should either name the tie or print the script's own choice.

---

## B. Cuts (machinery only)

Each cut names what the owner keeps. None removes a user-facing feature.

### B1. `serve --test-proto N`

A production flag that makes the daemon lie about its protocol major so clients' mismatch
banners can be tested. The banners can be tested from a fixture frame instead (P1-05 already
ships `contracts/ipc/v1/hello.ndjson`; `Store.js` and `api::Client` consume it in P2-02 and
P1-11). Keeps: the banner, the `proto_unsupported` error, the test. Removes: a flag, a branch in
`hello`, and the possibility of a test build answering wrongly in production.

```diff
@@ §3.2.1 (also B2) @@
-under systemd). Flags: `--detach` (used only by the no-systemd fallback, §3.2.14),
-`--idle-exit-secs N`, `--no-notify` and `--test-proto N` (all for tests; `--test-proto` makes the
-daemon claim another protocol major so clients' mismatch banners can be exercised). It appears in
+under systemd). Flags: `--idle-exit-secs N` and `--no-notify` (both for tests). The clients'
+protocol-mismatch banners are exercised from the fixture `contracts/ipc/v1/hello.ndjson` (an
+`s2c` `hello` with `proto: 2`), which `Store.js` and `api::Client` consume (P1-05, P2-02), so the
+daemon needs no flag to lie about its version. It appears in
```

```diff
@@ P1-03 Do @@
-  (§3.2.1: flags `--detach`, `--idle-exit-secs N`, `--no-notify`, `--test-proto N`). `run_serve()`
+  (§3.2.1: flags `--idle-exit-secs N`, `--no-notify`). `run_serve()`
```

```diff
@@ P1-06 Do @@
-  `--test-proto N` makes `hello` claim major `N`; the dispatch pool (8 threads, queue 64, then
+  the dispatch pool (8 threads, queue 64, then
```

```diff
@@ Appendix F serve row (also declares 70/75 for agents) @@
-| `serve [--detach] [--idle-exit-secs] [--no-notify] [--test-proto]` | is the daemon | n/a | n/a | 0, 1 (`already_running`), 70, 75 |
+| `serve [--idle-exit-secs] [--no-notify]` | is the daemon | n/a | n/a | 0, 1 (`already_running`), 70 (panic), 75 (binary replaced while idle); `capabilities.exit_codes` notes that these two belong to the unit, not to the 0..8 dictionary |
```

### B2. `serve --detach`, the double fork, the rotated `daemon.log`, `last-daemon.json`, `server.launcher: detached`

The no-systemd fallback (§3.2.14, R13 "low/low") self-daemonises with a double fork, `setsid`, a
1 MiB×3 log rotation, and a client-side memory of the last log path. The owner's desktop and SSH
logins both have a user manager (`pam_systemd`; verified: `systemctl --user show-environment`
exports `WAYLAND_DISPLAY` and `DBUS_SESSION_BUS_ADDRESS`), and the test harness runs `serve` as a
foreground child (`daemon.py --no-systemd`), so nothing on this machine or in CI executes that
path. A daemonisation code path is the kind that has its own bugs (stdio leaks, orphaned
children, a rotation race) and no test ever runs it. Keeps: running the daemon anywhere
(`fbittorrent serve` in a terminal or tmux window; every client attaches), and the hint that says
so. Removes: five flags and files and one `hello` enum value.

```diff
@@ §3.2.4 steps 4–5 @@
-4. Otherwise spawn `fbittorrent serve --detach` and poll `hello` for up to 15 s.
-5. On failure: exit 4 with the hint `journalctl --user -u fbittorrent -n 50` (or the fallback log
-   path). The client remembers the last `server.log` it saw (A.2) in
-   `~/.local/state/fbittorrent/last-daemon.json`, so the hint names the right log even when nothing
-   answers.
+4. Otherwise (no user manager, §3.2.14): exit 4 with the hint `fbittorrent serve` ("run the daemon
+   in a terminal or a tmux window; this session has no systemd user manager").
+5. On failure: exit 4 with the hint `journalctl --user -u fbittorrent -n 50`.
```

```diff
@@ §3.2.14 @@
-Rare (a TTY login without `pam_systemd`, a container). Detected when
-`systemctl --user show-environment` fails. The client then spawns `fbittorrent serve --detach`
-(double fork, `setsid`, stdio to `~/.local/state/fbittorrent/daemon.log`, rotated at 1 MiB with
-three files kept). Everything else behaves the same; only restart-on-crash is lost.
+Rare (a TTY login without `pam_systemd`, a container; never the owner's desktop or an SSH login,
+where `pam_systemd` starts the user manager). Detected when `systemctl --user show-environment`
+fails. A command that would start the daemon exits 4 with the hint `fbittorrent serve`; the owner
+runs the daemon in a terminal or tmux window and every client attaches to it. There is no
+self-daemonising path (double fork, log rotation, a remembered log path), because the only
+environment that would use it is one the owner does not have, and the test harness runs `serve`
+as a foreground child (P1-07, `daemon.py --no-systemd`).
```

```diff
@@ §3.2.7 diagram and table @@
-                 systemctl start / serve --detach
+                 systemctl start / serve (terminal)
```

```diff
@@ §3.2.7 transition table @@
-| Absent → Starting | `systemctl --user start`, `serve --detach`, or `Restart=on-failure` after a crash | unit active; `hello` answers with `state: "starting"` |
+| Absent → Starting | `systemctl --user start`, `fbittorrent serve` in a terminal (§3.2.14), or `Restart=on-failure` after a crash | unit active; `hello` answers with `state: "starting"` |
```

```diff
@@ P1-04 Do @@
-  `hello` on `api.sock`; the 200 ms connect timeout, the 30 s `systemctl --user --no-ask-password
-  start fbittorrent.service` bound, the `deactivating` wait of step 3, the `serve --detach`
-  fallback of §3.2.14, the hint of step 5 and `~/.local/state/fbittorrent/last-daemon.json`). New
+  `hello` on `api.sock`; the 200 ms connect timeout, the 30 s `systemctl --user --no-ask-password
+  start fbittorrent.service` bound, the `deactivating` wait of step 3, the exit-4 hint of step 4
+  when there is no user manager (§3.2.14) and the hint of step 5). New
```

```diff
@@ A.2 result (also answers the P1-06 probe's "where does instance come from") @@
-| result | `proto: u64`, `server: {version: str, build: str, pid: u64, launcher: enum(systemd\|detached\|foreground), log: str}` (`log` is `journalctl --user -u fbittorrent` or the file path, ready to print as a hint), `engine: str`, `instance: str` (uuid), `engine_epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `caps: [str]` (§3.4.3), `limits: {line_in: u64, line_out: u64, inflight: u64, subs: u64, hashes: u64}`, `policy: {stop: enum(idle\|last-ui\|explicit), idle_exit_secs: u64}` |
+| result | `proto: u64`, `server: {version: str, build: str, pid: u64, launcher: enum(systemd\|foreground), log: str}` (`launcher` is `systemd` when `NOTIFY_SOCKET` was set at start; `log` is `journalctl --user -u fbittorrent` under systemd and `the terminal running fbittorrent serve` otherwise, ready to print as a hint), `engine: str`, `instance: str` (32 lowercase hex characters from 16 bytes of `/dev/urandom`, read with `std::fs`; no uuid crate), `engine_epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `caps: [str]` (§3.4.3), `limits: {line_in: u64, line_out: u64, inflight: u64, subs: u64, hashes: u64}`, `policy: {stop: enum(idle\|last-ui\|explicit), idle_exit_secs: u64}` |
```

### B3. The timer-wheel runner retargeting (P0-02 step 5)

P0-02 spends a paragraph keeping `regression_tests/timer_wheel_tests.rs` and
`run_timer_wheel_tests.py` alive after the vendored tree leaves: `cargo metadata` discovery, a
copy of two source files into `target/timer-wheel/`, a support `Cargo.lock` fixture rescued from
the archive before P0-01 runs. The P0-02 probe in round 3 already found that the runner's
`select_core` "accepts only a registry or vendored source path", so in a clean clone it can never
pass. The patch's tests (63 timer-wheel tests, `region_owner_tests`) live in the fork and run
there in P0-02 step 3 (`cargo test --all-features`), which is the test of record. Keeps: the
tests of the patch, where the patch is. Removes: two files from C5, one fixture directory, a
`#[path]` trick and the only reason P0-02 and P0-01 fight over a file.

```diff
@@ P0-02 Do, step 5 @@
-  Retarget the timer-wheel runner, which today hard-codes the vendored path
-  (`regression_tests/timer_wheel_tests.rs:3,65`, `run_timer_wheel_tests.py:55-57,112-113`) and
-  defaults `--support-lock` to a `target/cpu-optimization/` file that does not exist
-  (`run_timer_wheel_tests.py:83-84`): the runner finds the asupersync source directory with `cargo
-  metadata` (`packages[].manifest_path`), copies `src/time/wheel.rs` and
-  `wheel_metamorphic_tests.rs` into `target/timer-wheel/src/time/` where the `#[path]` of
-  `timer_wheel_tests.rs` now points, and reads its support lock from
-  `regression_tests/fixtures/timer-wheel/support-Cargo.lock`, copied in this item from
-  `docs/validation/2026-10-11/cpu-optimization/timer-wheel-tests/support-Cargo.lock` before P0-01
-  archives it (if P0-01 ran first, restore that one file from the archive). (6) Archive the
+  Retire the timer-wheel runner instead of retargeting it: delete
+  `regression_tests/timer_wheel_tests.rs` and `run_timer_wheel_tests.py` from the tree C5 commits
+  (they hard-code the vendored path, `timer_wheel_tests.rs:3,65`, `run_timer_wheel_tests.py:55-57,112-113`,
+  default `--support-lock` to a file that does not exist, `:83-84`, and can never pass from a
+  clean clone because `select_core` accepts only a registry or vendored rlib). The patch's 63
+  timer-wheel tests and `region_owner_tests` live in the fork and run there in step (3); the
+  receipt records that run, and P0-01's archive keeps the receipt-era runner. (6) Archive the
```

```diff
@@ P0-02 Accept @@
-  --exclude=.cargo-ok` between the fork checkout at the tag and the extracted archive is empty;
-  `run_timer_wheel_tests.py` passes 63 tests from the retargeted source.
+  --exclude=.cargo-ok` between the fork checkout at the tag and the extracted archive is empty;
+  `cargo test --all-features` in the fork checkout passes the 63 timer-wheel tests and
+  `region_owner_tests` (recorded in the receipt); the two runner files are absent from C5.
```

```diff
@@ P0-02 Tests @@
-  §9.1 / E.1 shape to `target/test-logs/asupersync/{build,test,timer_wheel}.jsonl` (one step per
-  command with its exit code and duration, `final` lines; the worker-panic scan of `all.sh:12-16`
-  applies); `run_timer_wheel_tests.py` green with 63 tests.
+  §9.1 / E.1 shape to `target/test-logs/asupersync/{build,test,fork_test}.jsonl` (one step per
+  command with its exit code and duration, `final` lines; the worker-panic scan of `all.sh:12-16`
+  applies).
```

(Follow-on, no hunk: §8.1's C5 title drops "timer-wheel tests"; R19's third clause goes.)

### B4. The `last-ui` and `explicit` stop policies (conditional on Q1)

Three policies, each with its own daemon cases (`idle_exit_last_ui`, `explicit_never`), a
`last-ui` branch in `policy.rs`, a `q`-asks-first branch in the TUI (P1-15) and the popup, an enum
in `Settings`, `hello.policy` and the busy table's second column. The owner chose `idle` (A1);
`last-ui` is the behaviour they are leaving (D9), and `explicit` is `idle_exit_secs = 0`. If Q1
confirms `idle`, the two alternatives are settings nobody will set. Keeps: every behaviour the
owner sees, plus "never exit" through `idle_exit_secs = 0`. Removes: an enum, two cases, two UI
branches. Apply only after Q1; the hunks below are the canonical lines, and the integrator greps
`last-ui` (12 hits: §0.3 A1, §2.6, §3.2.10 table, A.2, A.9 `Settings`, P1-15, P3-15, DL-02).

```diff
@@ §3.2.10 bullets @@
-- `last-ui`: as `idle`, and additionally the daemon drains and exits when the interactive count
-  goes from one or more to zero, even with downloads active. The closing UI asks first when
-  transfers are active (P1-15).
-- `explicit`: the daemon never exits on its own.
+- `idle_exit_secs = 0` means "never exit on its own" (the former `explicit` policy); the former
+  `last-ui` policy is not implemented: it is the behaviour the owner is leaving (closing a
+  window kills downloads, D9), and §10.3 Q1 asks once; if the owner wants it, it returns as a
+  P7 item with its two cases, not as a setting nobody sets.
```

```diff
@@ P1-12 Do @@
-- *Do:* `src/daemon/policy.rs` (§3.2.10): the config keys `stop_policy = "idle" | "last-ui" |
-  "explicit"` (default `idle`) and `idle_exit_secs` (default 300, minimum 30; `serve
+- *Do:* `src/daemon/policy.rs` (§3.2.10): the config key `idle_exit_secs` (default 300, minimum 30,
+  `0` = never exit on its own; `serve
```

```diff
@@ P1-12 Do, end @@
-  {reason: "idle"}` and exit 0; `last-ui` also drains when the interactive count drops to 0.
+  {reason: "idle"}` and exit 0.
```

```diff
@@ P1-12 Accept @@
-  local download runs; stays while an interactive client is connected; exits with only a bar
-  client; stays while parked with a waiting download; `last-ui` exits when the TUI client leaves;
-  `explicit` never exits; `busy_reasons` names the reason in every "stays" case.
+  local download runs; stays while an interactive client is connected; exits with only a bar
+  client; stays while parked with a waiting download; `idle_exit_secs = 0` never exits;
+  `busy_reasons` names the reason in every "stays" case.
```

### B5. The `ping` operation

No client uses it: liveness is `hello` (clients) and `heartbeat` (server), and `app.info` answers
anything `ping` would. Keeps: everything. Removes: one row and one handler.

```diff
@@ A.4 @@
-| `ping` | none | `{ts: ts}` | no | P1-06 |
```

```diff
@@ P1-06 Do @@
-  operations `hello`, `ping`, `app.info`, `app.client`, `app.shutdown`, `app.restart`,
+  operations `hello`, `app.info`, `app.client`, `app.shutdown`, `app.restart`,
```

### B6. The per-torrent speed-history rings (P6-03)

"120 per visible torrent" samples have no consumer: §4.7's details tabs draw no per-torrent
graph (the qBittorrent "Speed" tab became the global status-row sparkline, §4.7), §5.6 has none,
and `app.stats.history` is global. Keeps: the global 600-sample ring, the sparkline in both UIs,
`status --json`'s `history`. Removes: a per-torrent sampler and its memory.

```diff
@@ P6-03 Do @@
-- *Do:* §6.3 row "Speed history": in the daemon a ring of 600 one-second global samples
-  (`download`, `upload`) and 120 per visible torrent; `Summary.history: [u64]` (the last 60
-  download samples, A.9) and `app.stats {history: true}` → `Stats.history {download, upload,
-  interval_s: 1}` (A.4, A.9); the `stats` topic (A.7, 5,000 ms); `status --json` gains `history`
+- *Do:* §6.3 row "Speed history": in the daemon one ring of 600 one-second global samples
+  (`download`, `upload`; no per-torrent rings: nothing in §4.7 or §5.6 draws one, and the
+  qBittorrent "Speed" tab became the global sparkline, §4.7); `Summary.history: [u64]` (the last 60
+  download samples, A.9) and `app.stats {history: true}` → `Stats.history {download, upload,
+  interval_s: 1}` (A.4, A.9); the `stats` topic (A.7, 5,000 ms); `status --json` gains `history`
```

```diff
@@ §6.3 Speed history row @@
-| Speed history (P6-03) | Ring of 600 one-second global samples (`download`, `upload` bytes per second) and 120 per torrent with a visible details view | `Summary.history: [u64]` (the last 60 download samples), `app.stats {history: true}` → `Stats.history {download, upload, interval_s: 1}`; topic `stats` (5,000 ms); cap `history` | ftui `sparkline` in the tab bar (§5.8), redrawn on data | `components/Sparkline.qml` in the status row (§4.6), no interpolation (§4.13) | `status --json` gains `history` | Unit `history::tests::ring_bound_and_order`; TUI golden at 120×30 with a throttled ramp; `qs-smoke.sh::history_points`; `agent_contract.py::status_history_field` |
+| Speed history (P6-03) | One ring of 600 one-second global samples (`download`, `upload` bytes per second); no per-torrent ring (no view draws one) | `Summary.history: [u64]` (the last 60 download samples), `app.stats {history: true}` → `Stats.history {download, upload, interval_s: 1}`; topic `stats` (5,000 ms); cap `history` | ftui `sparkline` in the tab bar (§5.8), redrawn on data | `components/Sparkline.qml` in the status row (§4.6), no interpolation (§4.13) | `status --json` gains `history` | Unit `history::tests::ring_bound_and_order`; TUI golden at 120×30 with a throttled ramp; `qs-smoke.sh::history_points`; `agent_contract.py::status_history_field` |
```

Considered and kept: `app.info.clients` and `.subscriptions` (cheap, and P2-06/P3-10 acceptance
reads them); the `events` replay ring (agent-facing); `qs-visual.sh --compare` (the owner asked,
Q23); the pixel-diff percentage (one ImageMagick call); `FBITTORRENT_UNIT`, `FBT_TEST_*` hooks
(test isolation); the resource gate (DL-18); `handler = "confirm"` and `middleClick` (settings the
plan itself calls user-facing; Q9 can retire the first).

---

## C. Improvements

### C1 (P1). Hide the popup's key-hint row at rest

**Analysis.** The plan's own token table admits "no first-party popup renders a key-hint row
(`network`, `bluetooth`, `tailscale`, `agents`: none)" and keeps one anyway, as a caption line.
Verified against the installed panels: the network and bluetooth popups end with their last
section (`PanelSectionHeader` rows, `PanelSeparator`s); nothing resembles a footer. For the
owner's stated goal ("mistaken for a first-party panel"), this row is the single structural tell
in the resting popup; every other element in §4.5's mockup (hero with meta and trailing
controls, separator, section headers, 28 px rows) has a first-party twin. The hint matters to the
keyboard user, who has already pressed a key. Showing the row from the first key press of an
opening keeps discoverability where it is needed and makes the mouse user's popup structurally
identical to the Wi-Fi panel, which is what the `--compare` pair image will be judged on.

```diff
@@ §4.2.3 Key hint row @@
-| Key hint ("a add · / search · w window") | `Style.font.caption`, one line, rendered like `PanelSectionHeader` (bold caption, letterSpacing 1.2, no uppercase) | keys in foreground, labels dim; accent only when it passes §4.2.1 | no first-party popup renders a key-hint row (`network`, `bluetooth`, `tailscale`, `agents`: none); omaqbt's table uses body-size hints (`TorrentTable.qml:324-338`); the popup keeps it to one caption line so it reads as section chrome, not a toolbar (Sprout's "card-dashboard" anti-pattern) |
+| Key hint ("a add · / search · w window") | `Style.font.caption`, one line, rendered like `PanelSectionHeader` (bold caption, letterSpacing 1.2, no uppercase); **hidden at rest**, shown from the first keyboard key the popup receives until it closes, and always shown in the dashboard | keys in foreground, labels dim; accent only when it passes §4.2.1 | no first-party popup renders a key-hint row (`network`, `bluetooth`, `tailscale`, `agents`: none), which is the one structural tell in the resting popup; hiding it until the keyboard is used keeps the mouse user's popup identical in structure to the Wi-Fi panel (hero, separator, section headers, rows) and keeps discoverability for the keyboard user; omaqbt's table uses body-size hints (`TorrentTable.qml:324-338`); the popup keeps it to one caption line so it reads as section chrome, not a toolbar (Sprout's "card-dashboard" anti-pattern) |
```

```diff
@@ P2-03 Do @@
-  in place to at most 10 rows, the `KeyHint` caption row ("a add · / search · w window · t terminal
-  · z turtle", §4.2.3), the inline confirm strip of §4.9 ("Remove '<name>'? Files are kept."
+  in place to at most 10 rows, the `KeyHint` caption row ("a add · / search · w window · t terminal
+  · z turtle", §4.2.3; hidden until the first key press of this opening, so the popup at rest has
+  the Wi-Fi panel's structure), the inline confirm strip of §4.9 ("Remove '<name>'? Files are kept."
```

(The §4.5 and Appendix C mockups then show the keyboard-mode popup; a one-word caption on each
says so.)

### C2 (P1). Say what the contract-1.x torrent prints when the API value is absent

**Analysis.** P1-11 keeps the qBittorrent-shaped `qbt::Torrent` (`src/qbt.rs:11-36`) for
`list`/`show`/`status`/`wait`, rebuilt from A.9. Three of its fields are non-optional numbers
whose truthful value is now "absent": `eta` (today `8640000`, `src/engine.rs:1477`), `num_seeds`
(today `0`, `:1480`), `progress` (today `progress_known: false`). A 1.x parser (`serde` with
`i64`, or `jq '.eta'`) breaks on a missing key, so byte-compatibility means keeping qBittorrent's
own sentinels in the old fields and putting the truth in the new-name fields. The plan says the
additions are "new fields with new names" but never says what the old fields print; the probe for
P1-11 in round 3 asked exactly this and the fix landed the shaper without the rule.
`capabilities.changes[]` and `robot-docs` must say which field to read, or the agent contract
teaches agents the sentinel. Also: `capabilities` should declare `serve`'s 70/75 as unit exit
codes outside the 0..8 dictionary (folded into B1's Appendix F hunk).

```diff
@@ P1-11 Do, shapes @@
-  `TorrentView::to_legacy_json()`, P1-19); the additions are new fields with new names:
-  `native_state` (the §3.5 enum), `reason`, `checked`, `origin`, later `playable`; `state` keeps its
-  qBittorrent string. `warnings: [{code, message}]` is added at the top level of every JSON
+  `TorrentView::to_legacy_json()`, P1-19); the additions are new fields with new names:
+  `native_state` (the §3.5 enum), `reason`, `checked`, `origin`, `eta_s` (absent when unknown) and
+  `seeds` (P5-01), later `playable`; `state` keeps its qBittorrent string, and the old numeric
+  fields keep qBittorrent's own sentinels so a 1.x parser never meets a missing key: `eta` is
+  `8640000` when `eta_s` is absent (qBittorrent's documented "infinity"), `num_seeds` stays `0`
+  until P5-01 fills it and `num_leechs` is the peer count, `progress_known: false` with
+  `progress: 0` when `progress` is absent (the parked shape today); `capabilities.changes[]`
+  (1.2.0) says so under `field: "list[].eta"` and `field: "list[].num_seeds"`, and `robot-docs`
+  tells agents to read `eta_s`/`seeds`/`native_state`. `warnings: [{code, message}]` is added at the top level of every JSON
```

### C3 (P2). One unit rule for sizes

**Analysis.** §4.2.3 says binary units ("1.4 GiB"); the add sheet, remove dialog, disk banner,
stats footer and destination row print "GB" (lines 1769, 2120, 2671, 2678, 5030, 5033, 5162,
5320, 6441–6464). `Model.js` is the one formatter, so this is a spelling rule, not code; fixing
it now keeps the fixture (`contracts/ui/format.json`) and the captures from arguing.

```diff
@@ §4.2.3 Numbers @@
-Numbers are formatted once, in `Model.js`, and used by every surface: sizes with one decimal and
-binary units up to 100 GiB ("1.4 GiB"), rates "4.2 MB/s" (decimal, like the bar's other widgets),
+Numbers are formatted once, in `Model.js`, and used by every surface: sizes with one decimal and
+binary units up to 100 GiB ("1.4 GiB" in sentences, dialogs, banners and the add sheet; the
+compact "1.4G" only in table cells; never "GB", which several mockups in this plan still print and
+the captures of P2-08 and P3-16 correct), rates "4.2 MB/s" (decimal, like the bar's other widgets),
```

### C4 (P2). A licence file at the repository root

**Analysis.** `plugin/oliver.torrents/manifest.json` declares `"license": "MIT"`; the repository
root has no `LICENSE` file (`ls LICENSE*` is empty). P0-03 checks nova3's headers and asks about
the Asupersync rider, but the product's own licence is never written down before the first
commit. Cheap, and it is the file the rider conversation (Q3) will point at.

```diff
@@ P0-03 Do @@
-  allow embedding in an MIT binary (`include_str!`, `src/search.rs:37-47`), stop and ask the owner.
+  allow embedding in an MIT binary (`include_str!`, `src/search.rs:37-47`), stop and ask the owner.
+  Add `LICENSE` at the repository root (MIT, the licence `plugin/oliver.torrents/manifest.json`
+  already declares; today no root licence file exists) and a `NOTICE` line pointing at
+  `vendor/rqbit/LICENSE` (Apache-2.0) and, under option B, `vendor/asupersync/LICENSE` with its
+  rider; with the fork, the rider travels with the fork and the README names it.
```

### C5 (P2). Q3 should name the rider's third clause

**Analysis.** Q3 names two facts (the rider travels with every distribution; the private
repository would need git auth). The rider also forbids making the Software or a derivative
"available to" the restricted parties and counts "analyzing" and "testing" as use
(`vendor/asupersync/LICENSE`, verbatim). Agents from both named parties read `vendor/asupersync/`
and the binary that links it in every phase of this plan. Whether that reading is the owner's
risk to take is the owner's call, but the question should be in front of them with all three
facts, not two.

```diff
@@ §10.3 Q3 @@
-| Q3 (**answer before P0-04 commits C3**: it decides what goes into history) | May a public fork `odcpw/asupersync` hold the patch (about 1,300 lines over 10 files, P0-02; you create the repository, since publishing is your act; builds need network once), or should the full 56 MB `vendor/asupersync/` tree be committed (§8.2 option B, commit C0; no second repository, no network, 56 MB in every clone)? Two facts bear on it: `vendor/asupersync/LICENSE` is "MIT License (with OpenAI/Anthropic Rider)", which every distribution must carry unmodified and which restricts the named parties (the plan draws no legal conclusion; the fork is a distribution and so is option B); and `odcpw/fbittorrent` itself is private, so a private fork would need git authentication for cargo and CI (`net.git-fetch-with-cli`), which the plan does not cover. | the public fork; the full tree as the fallback |
+| Q3 (**answer before P0-04 commits C3**: it decides what goes into history) | May a public fork `odcpw/asupersync` hold the patch (about 1,300 lines over 10 files, P0-02; you create the repository, since publishing is your act; builds need network once), or should the full 56 MB `vendor/asupersync/` tree be committed (§8.2 option B, commit C0; no second repository, no network, 56 MB in every clone)? Three facts bear on it: `vendor/asupersync/LICENSE` is "MIT License (with OpenAI/Anthropic Rider)", which every distribution must carry unmodified and which restricts the named parties (the plan draws no legal conclusion; the fork is a distribution and so is option B); the rider also forbids making the Software or a derivative "available to" those parties and counts "analyzing" and "testing" as use, and this plan is executed by agents of both parties that read `vendor/asupersync/` and the binary that links it, so the question is not only where the patch lives but whether you accept that reading of the rider for the whole project; and `odcpw/fbittorrent` itself is private, so a private fork would need git authentication for cargo and CI (`net.git-fetch-with-cli`), which the plan does not cover. | the public fork; the full tree as the fallback |
```

### C6 (P2). The dashboard looks like an app, and that is the right call; one word in the plan should say so

No hunk. §4.6's window has numbered tabs (`1 Torrents 2 Search …`), a bottom key row and a
sidebar with counts. No first-party panel has any of these; the only first-party
`FloatingWindow` (`dev-gallery`) has a scrollable page with a one-line explanatory caption. The
window is not a popup and will not be mistaken for one; what makes it Omarchy-native is what §4.6
already specifies: `Commons.Color.background`, `Border.controlSpec("normal")` pane frames,
`FilterInput`, `ConfirmDialog`, `ToggleSwitch`, `Dropdown`, `Style` spacing and the monospace
family. The round-3 reviewers settled the geometry; nothing here needs changing.

### C7 (P2). The TUI at 80 columns on the monochrome theme

No hunk beyond A20. Checked the §5.6 80×24 mockups against the owner's `colors.toml` with the
§4.2.1 ladder: text `#ffffff` (21:1), dim `#b6b6b6` (10.4:1), the selected row carries `▌`, every
state is a glyph plus a word in a 9-cell column, errors are bold `!`. `‖` (U+2016) has East Asian
width "A" (ambiguous); Ghostty in a non-CJK locale renders it in one cell, and the
`state_glyph_width_one` test guards it. Readable as specified.

---

## D. The first demo, end to end: a magnet click at M1a with no terminal

Premise: a clean Omarchy machine (this one, re-imaged), the plan applied through M1a
(P0-01…P1-19 except P2-*), the owner's data restored. Every step lists who acts, what the plan
says happens, and where the plan is silent or wrong (**gap**).

| # | Actor | Step | Plan says | Gap |
|---|---|---|---|---|
| 1 | owner | clone, `cargo build --release --locked` | §8.1 step 4 proves a clean clone builds (needs network once for the Asupersync fork, §8.2) | none, once Q3 is answered and P0-02/P0-04 are merged |
| 2 | owner | install the binary | §9.7 M1a step (1): `scripts/install.sh` | **A3**: the script is P2-07's, after M1a; a `cp` fails with `ETXTBSY` while the v0.1 TUI runs the old binary. With A3 the script exists at P1-04 and runs `setup --magnet` |
| 3 | `setup` | write the unit, `daemon-reload`, the desktop entries (`Exec=fbittorrent add --origin handler %u`, `Icon=fbittorrent`), `xdg-mime default` | P1-04, P1-11, P1-13 | none after A3 (the plan never said *when* `setup` runs at M1a) |
| 4 | owner | quit the v0.1 TUI with `Q` | §3.6, M1a step (1): reads work through the old `engine.sock`, writes are refused | none (P1-11 implements the probe order; the receipt records it) |
| 5 | browser | hands `magnet:…` to `fbittorrent-magnet.desktop` | `src/setup.rs:113-115`, P1-11's `--origin handler` | none |
| 6 | `fbittorrent add` | `ensure_daemon()`: `hello` fails, unit exists, `systemctl --user --no-ask-password start fbittorrent.service` | §3.2.4 steps 1–3 | none |
| 7 | systemd | starts `fbittorrent serve`; `READY=1` once `api.sock` listens and the slot is `Starting` | §3.2.2, §3.2.4 | none |
| 8 | daemon | restores the ledger in background tasks (P0-09), the session exists, slot `Running` | §3.2.4, DL-49 | none |
| 9 | `fbittorrent add` | `torrents.add {magnet, origin: handler}` while the slot may still be `Starting` | §3.2.4: waits up to 60 s for the session; §9.3 preamble: `engine_unavailable` at once | **A2**: the two rules disagree; the click fails under the preamble's reading |
| 10 | daemon | the ledger record exists from the magnet's hash and `dn=` (P1-09), row `metadata` | P1-09 | none |
| 11 | daemon or CLI | post "Added" with Undo and Open window | §3.8 / P1-13: both the daemon and the CLI are said to post it | **A5**: decide the daemon (handler origin only); the CLI's plain toast stays for TTY use |
| 12 | notifier | `notify-send -a fbittorrent -i fbittorrent -A undo=Undo -A window="Open window" Added "<name>"` (blocks until closed) | §3.8, verified `-A` implies `--wait` | none; 4 notifier threads bound it |
| 13 | owner | clicks "Undo" | `torrents.remove {delete_files: false}` from the daemon | none |
| 14 | owner | clicks "Open window" | `fbittorrent dashboard` → no `panel` kind at M1a → falls back to `tui` via `omarchy-launch-or-focus-tui` | none (documented in P1-04, Appendix F) |
| 15 | old popup | `fbittorrent status --json` every 10 s (2 s while open) | §0.6: "works through the compat socket" | **A4**: it works through the CLI; the running shape must keep the 1.x fields (P1-11 hunk) |
| 16 | old popup | icon visibility while the daemon runs | `online = status.ok === true` | none while running |
| 17 | daemon | download completes, `torrent.completed`, "Download complete" with Open / Show in folder | P1-13 | none |
| 18 | owner | `fbittorrent tui`, `q` | P1-15: one-time notice, download continues | none |
| 19 | daemon | idle countdown (300 s), `bye {reason: idle}`, exit 0 | P1-12 | none |
| 20 | old popup | next `status --json` returns `{ok: true, running: false, …}` | DL-22 | **A4**: `online` stays true, the icon never hides until P2-03; one-line fix in P0-07 |
| 21 | agent | `fbittorrent events --follow --json` in step (5) of the demo | P1-18 | none |
| 22 | owner | `systemctl --user is-active fbittorrent` → `inactive` | §9.7 step (6) | none |

Also checked and sound: nothing starts at login (no `[Install]`, verified `systemctl --user show`
defaults); the notification icon name resolves through hicolor (P1-13 installs it); the user
manager exports `WAYLAND_DISPLAY` and `DBUS_SESSION_BUS_ADDRESS` (verified), so `notify-send` and
`xdg-open` work from the unit; the start-limit comment in §3.2.2 matches this machine
(`DefaultStartLimitBurst=5`, `10s`).

---

## E. Self-containment sample: four fresh agents, one item each

Method: `r4/item-<id>.md` holds only the item's text (extracted by line range from §9.3:
P1-06 3501–3536, P1-08 3568–3601, P3-09 4410–4447, P4-04 4729–4747). Each went to a fresh
`general-purpose` subagent (model `opus`) with the prompt below, verbatim, followed by the item
between `===== ITEM BEGIN =====` / `===== ITEM END =====` markers and the instruction to deliver
through `SubagentHandback`:

> You are an implementing agent. Below is one work item from a plan, with no other context except the repository at /home/oliver/Projects/odcpw/fbittorrent (read-only for you) and the plan file docs/plan-2026-10-18/PLAN.md, which you may open only at the section anchors the item cites. Is this implementable? List every fact, file name, format, API shape, token, threshold or acceptance detail you would still have to guess, and say where you would expect it to be defined. Do not implement anything. Do not modify files. Answer in under 300 words.

Tally: 0 of 4 "implementable as written" (none will be, by the §9.3 preamble's design of anchoring
into canonical sections); 2 of 4 buildable from the cited anchors plus the repository (P1-06,
P3-09); 2 of 4 found plan-level facts that are wrong or absent (P1-08: the wake-up contract and
the missing ledger counter; P4-04: a contradiction on the first details tab and no goldens for
three screens). Every gap that is real is a hunk, here or in Section A.

### E.1 P1-06 (verbatim)

> **Verdict:** Mostly implementable. The framing, the limits, the op tables (A.4/A.5), the error codes (A.3) and `sort_key` are all specified. Below is what I would still have to guess, and where I'd expect each answer to be written down.
>
> **Missing or contradictory**
> 1. **`EngineOp` dependency:** the item runs every op through `EngineOp` (P1-19), but its "Depends on" lists only P1-03 and P1-05. That should be fixed in the item and in §9.4.
> 2. **Errors with no `id`:** the cited sections don't say what an error frame looks like when there is no request to answer. That covers the 33rd connection's `busy`, an oversized or unparseable line, `hello_required` and `bye`. Expected in A.1 and A.8 (not cited).
> 3. **Hello deadline:** when 5 s pass, does the server answer at once and close, or wait for the late frame? The `hello_after_5s` test assumes the latter. Also unclear: what `hello_twice` should return, and whether trickled bytes (`slow_hello_bytes`) count as speaking. Belongs in §3.4.3.
> 4. **`garbage_table` answers:** no mapping from each input to `invalid_message` or `invalid_args`, and none on whether `invalid_args` closes the connection. Undecided inputs: duplicate keys (serde keeps the last), `\r\n`, an empty line, a string `id`, an unknown top-level key.
> 5. **Replies over 1 MiB:** no rule for a server reply that would exceed 1 MiB, such as `torrents.list` or `torrents.files` on a large library. Belongs in §3.4.1.
> 6. **`--test-proto N`:** which command takes it, and does a client sending `proto:1` then succeed or get `proto_unsupported`?
> 7. **Features that arrive later:** what should P1-06 do with arguments and results owned by later items? These are `ref` (P1-09), `{url}` add and `delete_files` "pending" with no `mutation.result` yet, `stage`/`restore`/`origin`, `app.restart {scope: daemon}` (P1-17), and `set_upload_limit` "pending until P6-07". Accept, ignore, or `unsupported`?
> 8. **Timeouts:** what happens to a timed-out worker, a late result and its in-flight slot.
> 9. **Value sets left open:** `busy.details.limit` values, the `bye` reason values, the `unknown_op` "nearest" threshold, and the hint text for each error.
> 10. **Field sources:** how to detect `server.build`, `launcher`, `log`, `instance` (uuid, with no uuid crate in `Cargo.toml`), `policy`, `upgrade_pending`, and `app.network.interfaces`.
> 11. **Torrent mapping:** how `state` and `reason` are derived from engine states. That is §3.5 and A.10, neither cited.
> 12. **Wire types:** the format of `hash`, `ts`, `bytes` and `bps` (expected in A.1).
> 13. **Missing files:** neither the contract fixtures (`contracts/ipc/v1/*.ndjson`) nor `daemon.py` exist in the repo. I'd expect P1-05 to define the fixtures and §3.4.11 to say how a reply "matches" (exact values or types only). Also undefined: where `daemon.py` lives, how it starts an isolated daemon, and how `.events()` behaves.

Triage: 1 is transitive (P1-05 ← P1-19) and harmless to state; 2 is A13; 5 is A16; 6 is B1;
10's `instance`/`launcher`/`log` are in B2's A.2 hunk; 12 and 13 are answered by the Appendix A
preamble, P1-05, P1-07 and §3.4.11 (the agent was allowed only the cited anchors). Real and
derived here: 3, 4, 7, 8, 9, 10 (the rest), 11.

```diff
@@ P1-06 Do, head @@
-- *Do:* `src/api/server.rs` and `src/api/ops/{app,transfer,torrents}.rs` (§3.4.1 to §3.4.6,
-  §3.4.10, §3.9): the listener on `api.sock` in the runtime directory, bound under the lock
+- *Do:* `src/api/server.rs` and `src/api/ops/{app,transfer,torrents}.rs` (§3.4.1 to §3.4.6,
+  §3.4.10, §3.9; the wire types and envelope of A.1, the error event of A.8, the `state`/`reason`
+  derivation of §3.5 through P1-19's `TorrentView`): the listener on `api.sock` in the runtime directory, bound under the lock
```

```diff
@@ P1-06 Do, tail @@
   accepting, answer in-flight requests, `bye {reason}` last, close).
+  Rules the tests above assume: the 5 s `hello` deadline counts from `accept` and is not reset by
+  bytes without a newline; at the deadline the server sends `error {hello_required}`, `bye` and
+  closes; a second `hello` is `invalid_state`; `invalid_message` (not JSON, not an object, over
+  the cap, invalid UTF-8; `\r\n`-terminated and empty lines count as not JSON) always closes the
+  connection, `invalid_args` (`id` 0, negative, non-integer or reused; an unknown top-level key;
+  duplicate keys, which the server rejects by parsing into `serde_json::Map` with the
+  `preserve_order` check rather than letting serde keep the last) answers with `"id": null` and
+  keeps the connection; a timed-out op releases its in-flight slot at the timeout and its late
+  result is dropped; `busy.details.limit` is one of `connections`, `inflight`, `dispatch`,
+  `sessions`, `subs`; `unknown_op.details.nearest` lists names within Levenshtein 2 (P1-18);
+  arguments that later items add (`stage`, `restore`, `force`, `origin` values other than
+  `manual|handler|agent`) are `invalid_args` until those items land, and `ref` is accepted and
+  ignored until P1-09; `app.network.interfaces` is the `settings --interfaces` listing
+  (`src/main.rs`, the `Interfaces` arm); `upgrade_pending` is `false` until P1-17.
```

```diff
@@ P1-06 Depends @@
-- *Depends on:* P1-03, P1-05.
+- *Depends on:* P1-03, P1-05 (and through P1-05, P1-19's `EngineOp` and `TorrentView`).
```

### E.2 P1-08 (verbatim)

> Verdict: the protocol core (framing, rev, chunking, the 512-frame queue, heartbeat) can be built from §3.4.7 and A.6–A.8 as written. The item as a whole cannot be finished without guessing. Below is what I would have to guess, and where I'd expect each answer to be defined.
>
> 1. **Item shapes.** The fields of `Summary`, `Torrent`, `File`, `NetworkState` and `Settings`, and which fields count as "rate/progress" (coalesced) versus "structural" (sent at once). Expected in §3.5 and A.9/A.10, which the item does not cite. The `settings` topic also seems to come before P3-05 defines `Settings`.
> 2. **`caps` token.** The exact capability string and where it sits in `hello`. Expected in §3.4.3 and A.2.
> 3. **`since_seq` placement.** A.6's `subscribe` arguments leave it out, though A.7 uses it. A.7 also puts `details.oldest_seq` "in the result", but the result is `{sub, topics}`. Needs one fix in A.6.
> 4. **Ledger change counter.** It doesn't exist. `src/engine.rs:154` keeps an async asupersync `Mutex<Ledger>`, and the bus is a std thread. It's also unclear how live rates get read "under the ledger lock". Expected in P1-07.
> 5. **Fork accessor.** Its return type isn't given (`rqbit_runtime::sync::Notify` uses `notify_waiters`, which can miss a wake-up). Completion fires `finished_notify` (`live/mod.rs:910`), not `state_change_notify`, so the required completion test fails without a further fork change. "Playable" has no notify at all. At `:333` the notify fires before the state is set to `Error`.
> 6. **Engine restart.** Which topics are "torrent-like", and what happens to detail subscriptions whose hash is gone. §3.4.7 omits `reason`, which A.8 has.
> 7. **What `interval_ms` means** for "on change" topics (`network`, `settings`, `events`).
> 8. **Subscription limits.** The cap per connection and the total. Expected in §3.4.10.
> 9. **`resources.py --daemon`.** It doesn't exist yet. Its CLI, how it generates 100 fixture torrents and simulates subscribers, how it measures the `status --json` reference, and the CPU metric's units are all unspecified.
> 10. **`baselines/resources.json`.** Its schema and tolerance (whatever P0-06's gate reads), and the full path (the `baselines/` folder doesn't exist).
> 11. **NDJSON fixtures.** The location, format and replay harness for `subscribe-torrents.ndjson` and `subscribe-gap.ndjson`, and the `daemon.py` framework, which doesn't exist yet. Expected in §3.4.11, E.3 and P1-06.
> 12. **Log line format** for "slow client" (E.1).

Triage: 1 (coalesced set) is A15; 3 is A12; 4 and 5 are A14 (verified in the fork:
`finished_notify` at `live/mod.rs:201,910`; `state_change_notify` at `mod.rs:212`); 8 is A6; 2
and 11 are the cited anchors' neighbours (§3.4.3, §3.4.11, P1-05, P1-07); 1's `settings` worry is
answered by P1-05 typing `Settings` in P1. Real and derived here: 6, 7, 9, 10, 12 (7 is also in
the A6 `subscribe` hunk).

```diff
@@ P1-08 Do, tail @@
   baseline `regression_tests/baselines/resources.json` that P0-06's gate reads.
+  Spelled out: `resources.py --daemon --torrents N --subscribers K --seconds 60` (extending the
+  existing `regression_tests/resources.py`, which measures the TUI today) starts a disposable
+  daemon with N local fixture torrents (the P1-07 magnet fixture duplicated with distinct
+  payloads, paused, so the CPU is the bus's, not a transfer's), opens K `summary` subscriptions
+  at 5,000 ms, and records user+system CPU seconds from `/proc/<pid>/stat` and peak RSS from
+  `/proc/<pid>/status` (`VmHWM`) over 60 s; the reference is `fbittorrent status --json` run
+  every 10 s for 60 s against the same daemon, measured the same way on the CLI processes;
+  `resources.json` is `{"configs": {"<torrents>x<subscribers>": {"cpu_s": [3 runs], "rss_kb":
+  [3 runs]}}, "reference": {"cpu_s": [3 runs], "rss_kb": [3 runs]}}` (P0-06's gate reads the
+  spread of the three). "On change" topics (`network`, `settings`, `events`, `log`, the RSS and
+  Shows topics) ignore `interval_ms`. After `engine.changed`, every `torrents`, `summary` and
+  `torrent:<h>.*` subscription gets a fresh snapshot; a `torrent:<h>.*` topic whose hash no
+  longer exists gets `snapshot.begin {count: 0, reason: engine_restart}` and `snapshot.end`, and
+  the client closes it. The slow-client log line is `warn api slow client kind=<kind>
+  name=<name> queued=512` (§3.2.15).
```

### E.3 P3-09 (verbatim)

> **Verdict:** I could build the layout from the item as written, but not the acceptance tests or the contracts. I read only the cited sections (§4.3.2, §4.3.3, §4.6, §4.9–§4.12, Appendix B) and changed nothing.
>
> **What I would have to guess, and where it should be defined:**
>
> 1. **`Summary.counts`:** the key names, and which states fall into Queued, Done, Seeding and Errors. Only Downloading is broken down. Expected in P2-02/P2-08, §3.5 or A.9.
> 2. **Ready (P5-05) at P3:** whether it shows 0 or is left out. Should be §4.6.
> 3. **`contracts/ui/states.json`:** the file doesn't exist and its schema is unknown. Expected in §4.3.4 or the item that creates it.
> 4. **Where sort persists:** §4.3.2 says "plugin settings", §4.6 says `~/.local/state/fbittorrent/dashboard.json`. The file's schema, the sort-key enum, and which headers can be clicked (↑, PEERS and RATIO are not in the `s` cycle) are all open.
> 5. **`state().view` additions:** the base shape, how column names are spelled (`↓` or `down`), width units (px or space units), and the `toast` shape. Expected in §4.17.
> 6. **210 px vs `Style.space(210)`:** these differ when UI scale is not 1. The tier boundaries assume scale 1, and the test scale isn't stated.
> 7. **Spelling gaps:** the progress bar's cell count and block resolution; the pending spinner glyph and timeout; the banner component; the urgent `!` marker; the turtle and VPN pill glyphs and colours (§4.2.4 isn't cited); the message time format; and what clicking the last message opens if the Log view doesn't exist yet.
> 8. **Keys in scope at P3:** "every key of the Dashboard column" includes `z`, `R`, `P`, `K`/`J`, `U`, `X`, `e`, `[`/`]`, `L`, `,`, `?` and the add sheet, which belong to P5, P6 or other items. I'd need an explicit list.
> 9. **Small conflicts:** Esc order is dialog → filter → details → selection (§4.12), while Appendix B names only filter and selection. "Hover-cursor fill" isn't a shell token (`hoverFill` exists). Pane titles appear in neither mockup.
> 10. **Test harness:** I don't know whether `qs-dashboard.sh` and `qs-visual.sh` exist (presumably P3-08 creates them). Also missing:
>     - the name of the 200-torrent fixture;
>     - what generates the 1 Hz deltas;
>     - how to produce continuous wheel scrolling (not wtype);
>     - which render-timing number counts as the frame time, since Qt prints several;
>     - the "no JS loop per delta" lint (tool and rule);
>     - the `expect` model format and JSONL log line (Appendix E);
>     - whether `resizewindowpixel` needs a floating window;
>     - which states `qs-visual.sh` captures.
> 11. **Details pane:** its content is §4.7, which isn't cited, and the sheet's size isn't given.

Triage: 4 is A8; 6 is A22; 9's pane titles is A21; 3 is P2-08's (a dependency); 10's harness
existence is P3-08's (a dependency) and the `expect`/JSONL shapes are §3.4.11/E.1. Real and
derived here: 1, 2, 5, 7, 8, 10 (the rest), 11.

```diff
@@ P3-09 Do, keys and state @@
-  and does nothing at the top level (§4.12); every key of Appendix B's Dashboard column for this
-  view; `state().view` gains `columns: [{name, width, hidden}]`, `sort: {key, reverse}`, `toast`.
+  and does nothing at the top level (§4.12); the keys of Appendix B's Dashboard column whose items
+  have landed by P3 (move, page, first/last, `1`-`4`/Tab, `h`/`l`, `/`, Esc, `q`/`Q`, Enter and
+  double-click, `o`, `p`/Space, `u`, `d`/`x`/Delete, `D`, `s`/`S`, `f`, `i`/`w`, `y`; `a`/Ctrl+V
+  arrive with P3-11, `[`/`]` with P3-10, `?`, `,` and `L` with P3-15; `z`, `R`, `P`/`K`/`J`,
+  `U`/`X` and `e` with P5-04, P5-03, P6-05, P6-08 and P6-01); the sidebar buckets are
+  `Summary.counts` (A.9): Downloading = `downloading + stalled + metadata + checking`, the others
+  one key each, Ready present from the start and 0 until P5-05; `state().view` gains `columns:
+  [{name, width, hidden}]` (names `glyph, state, name, size, progress, down, up, eta, peers,
+  ratio, added`; widths in device pixels, the live `width` of the header cell), `sort: {key,
+  reverse}` (keys as the `s` cycle spells them, lowercase), `toast: {text, until} | null`; the
+  progress cell is eight block characters (`█▉▊▋▌▍▎▏` fractions, `Style.space(120)` minus the
+  percentage) and the pending spinner is the braille set `⠋⠙⠹⠸⠼⠴⠦⠧` at 8 fps (§4.10); the turtle
+  and VPN glyphs are §4.2.4's; the details slot renders `PaneFrame` with P3-10's empty state
+  until that item lands, and clicking the last message does nothing until P3-15's Log view.
```

```diff
@@ P3-09 Tests @@
-  (resizing through `hyprctl dispatch resizewindowpixel`); fixture `expect` models for the
-  sidebar counts; `qs-visual.sh dashboard <state>` captures.
+  (resizing through `hyprctl dispatch resizewindowpixel` after `hyprctl dispatch setfloating` on
+  the window, which the harness does first; the 200 torrents are the fixture state
+  `regression_tests/fixtures/states/table-200.json`, served by `daemon.py --serve-fixture
+  table-200`, whose seed/leech pair is throttled so rates change every second; wheel scrolling
+  is a `scroll(dy)` test hook added to §4.17 here, called 20 times per second, because `wtype`
+  has no wheel; the frame time is the `time in renderer` field of each `QSG_RENDER_TIMING` line;
+  the "no JavaScript loop per delta" lint is a `plugin/tests/lint.sh` rule: no `for`, `while` or
+  `.forEach(` inside the frame handlers of `Service.qml` and `views/*.qml`, loops live in
+  `Store.js` only); fixture `expect` models for the
+  sidebar counts; `qs-visual.sh dashboard <state>` captures.
```

### E.4 P4-04 (verbatim)

> Verdict: only partly. You could build the Torrents layout at 80 and 120 columns from the §5.6 mockups. Everything else needs guesses. I read only §5.5.3, §5.6, §5.8 and §4.11, plus the repo.
>
> Gaps:
>
> 1. **`contracts/ui/states.json`** does not exist. Its schema and state ids are in §4.3.4, which the item doesn't cite. It also doesn't say whether the file is read at runtime or embedded at build time, or who creates it.
> 2. **Files that don't exist yet.** `store.rs`, `mouse.rs`, `modals/` and `format.rs` are all missing (`src/tui/` has only `mod.rs`, `poster.rs`, `theme.rs`). I'd expect them from P4-02, P4-03 or §5.3. The shape of the rectangle and hit-test API, and whether clicking a header sorts, are undefined.
> 3. **Golden mechanics.** There is no golden harness and Cargo.toml has no dev-dependencies section. Undefined: file location and format, plain text or ANSI, the update command, and the fixture data the mockups show (names, speeds, "12:41 today", IMDb data).
> 4. **Goldens for Search, Shows and RSS at three sizes, and Torrents at 160.** No target mockups for these are in the cited sections (Appendix C isn't cited). `screens/01-torrents-160x45.txt` has no STATE column. Heights 24/30/45 are inferred.
> 5. **Column widths.** No widths are given for NAME, %, ↓ or ETA. It's unclear whether the `▌` marker sits inside the 9-cell state cell.
> 6. **"↓ when live" and blanks.** "Live" has no definition. The rule for "—" versus a blank cell is missing.
> 7. **Selected-torrent strip.** Its fields and truncation are undefined. The 80×24 mockup also shows a status line, which contradicts "medium adds the strip".
> 8. **Details modal.**
>    - First tab: "Files first" in the table, but the mockup opens on Overview.
>    - The Files, Peers and Log tabs have no mockups or data source.
>    - At medium: where the 100×30 modal sits, and what happens below 30 rows.
>    - Whether `[`/`]` wrap or stop at the ends.
> 9. **Banner.** §4.11 is the plugin's table. The TUI wording, height, priority between states and colours are missing. Disk states depend on P5-07, which comes after P4.
> 10. **Thresholds.** No list length for switching to `virtualized`. No scrollbar placement.
> 11. **Width check.** Which `display_width()` to use is unstated. It also overlaps `format::tests::states` in §5.5.3.
> 12. **Tmux smoke steps.** The steps and expected strings for `tui_smoke.sh` are not given.
>
> Minor: the crate is used through `ftui::widgets`; `ftui_widgets` isn't a direct dependency.

Triage: 8's first bullet is A10; 2 and 3 are P4-01's and P0-16's (dependencies); 12 is P4-10's;
7's "status line" at 80 is P4-02's footer row 2 (the selected item's name and context), not the
medium strip, which the hunk now says. Real and derived here: 1, 2 (the API), 4, 5, 6, 7, 8 (the
rest), 9, 10, 11.

```diff
@@ P4-04 Do @@
-  `store.rs`, §5.8), `virtualized` for long lists, `scrollbar` on every list; the details modal
-  `src/tui/modals/details.rs` (Overview · Files · Peers · Log tabs with `[`/`]`, full screen at
-  narrow, 100×30 at medium, on Enter at wide too; the 80×24 mockup of §5.6 is its golden); the
-  banner slot above the footer for §4.11 states (VPN, disk, stopped).
+  `store.rs`, §5.8), `virtualized` for every list (no threshold), `scrollbar` on the right border of
+  every list; the details modal `src/tui/modals/details.rs` (tabs Overview · Files at this item,
+  Log from P4-08, Peers from P5-02, Trackers from P6-01, Pieces from P6-02; `[`/`]` stop at the
+  ends; full screen at narrow, a centred 100×30 at medium that falls back to full screen under 30
+  rows, on Enter at wide too; the 80×24 mockup of §5.6 is its golden); the banner slot (one line
+  above the footer, the §4.11 popup wording, bold with a leading `!` on low-chroma themes,
+  priority disk > VPN > stopped/reconnecting > upgrade pending). Column widths at narrow: `▌` in
+  column 0, STATE 9, `%` 5, `↓` 6, ETA 6, NAME fills; medium adds SIZE 9 and PEERS 5, and the
+  one-line strip above the footer (state word · reason phrase of §4.3.4 · queue position · cap
+  text · path, middle-elided) is distinct from footer row 2, which at every width carries the
+  selected item's full name and context (P4-02). A cell is blank when the value does not apply
+  (`↓` and ETA when `dl_bps` is absent, §5.7 row 8) and `—` when it applies but is unknown (SIZE
+  before metadata). `contracts/ui/states.json` is embedded with `include_str!` (a contract, not
+  configuration); the rectangles are `layout::Rects { rows: Vec<(hash, Rect)>, headers:
+  Vec<(Column, Rect)>, tabs, footer }` recorded per frame and read by `mouse.rs` (a header click
+  sorts, as in §4.6); the width check uses `text.rs::dw` (`unicode-width`, the crate ftui uses)
+  and is the one `state_glyph_width_one` test, which §5.5.3 names. Goldens for Search, Shows and
+  RSS at the three sizes and for Torrents at 160 have no mockup in §5.6: they are generated from
+  the P0-16 fixture here and reviewed in the receipt; Appendix C's Search and Shows mockups at 80
+  are their targets.
```

---

## F. Grounding

### F.1 Thirty citations, re-verified on this machine

| # | Plan cites | Where in the plan | Found | Verdict |
|---|---|---|---|---|
| 1 | `src/main.rs:786-827` `run_native_app` | §0.1 | `fn run_native_app` at 786, closes at 827 | correct |
| 2 | `src/main.rs:452-455` TUI needs a TTY | §0.1 | `Cmd::Tui` 452, the TTY refusal 453–455 | correct |
| 3 | `src/main.rs:407-415` `status` closed shape `ok: false` | §3.2.13 | `"ok": false` at 410, `return Ok(Code::Ok)` 414 | correct |
| 4 | `src/main.rs:525-532`, `550-560` add diffs the list | §3.4.8 | `before:` sets at 525 and 550, `find(… !before.contains …)` at 531, 559 | correct |
| 5 | `src/main.rs:718-728` `act()` | §9.3 preamble | `fn act<F>` 718, JSON at 725 | correct |
| 6 | `src/main.rs:1296-1300` `-i qbittorrent` | §3.8, P1-13 | `fn notify` 1296, `"-i", "qbittorrent"` 1298 | correct |
| 7 | `src/engine.rs:432` `concurrent_init_limit` | §3.2.4 | line 432 | correct |
| 8 | `src/engine.rs:437` `fastresume: false` | D3 | line 437 | correct |
| 9 | `src/engine.rs:517-546` `Drop for AppEngine` | §3.2.2, D2 | 517–546; note the 4 s bound is shared: `shutdown_timeout(bound - elapsed)`, so the whole drain is ≤ 4 s, not "4 + 4" | correct citation; §3.2.2's "up to 4 s plus up to 4 s" overstates (harmless, `TimeoutStopSec=20`) |
| 10 | `src/engine.rs:830-832`, `831` "connect ProtonVPN" | §3.2.9, P7-09 | `bail!("connect ProtonVPN before adding a download")` 831 | correct |
| 11 | `src/engine.rs:1477` eta sentinel, `1480` `num_seeds: 0` | §3.4.9, D6 | `8640000` 1477, `num_seeds: 0` 1480 | correct |
| 12 | `src/engine.rs:262-277` `rebuild` | P5-03 | `async fn rebuild` 262–277 | correct |
| 13 | `src/engine.rs:154` async `Mutex<Ledger>` | A14 (this review) | `ledger: Arc<Mutex<Ledger>>` 154 | correct |
| 14 | `src/ipc.rs:136-164` `InstanceLock`, `:151` never unlinked | §3.2.5 | `impl InstanceLock` 136; the comment at 151 | correct |
| 15 | `src/backend.rs:62-86` `open_app` | §0.4, D12 | `pub fn open_app` 62–86 | correct |
| 16 | `src/agent.rs:186,221` "1 B/s" | D7 | both lines carry "1 B/s" | correct |
| 17 | `src/agent.rs:118-120` "try:" | §3.4.2 | `eprintln!("  try: {h}")` is line 121 | off by one; `:121` |
| 18 | `src/tui/theme.rs:66-67` muted then dark_foreground | D16 | 66 `muted`, 67 `dark_foreground` | correct |
| 19 | `src/tui/mod.rs:1699-1702` lifecycle polls | §0.1, §1.2 | 1699 `Msg::Poll`, 1700–1701 the checks | correct |
| 20 | `src/tui/mod.rs:193,282,1921-1924` `rss_initial_refresh_pending` | P3-07 | all four | correct |
| 21 | `vendor/rqbit/librqbit/src/torrent_state/mod.rs:212` `state_change_notify`, `:333,412-460,518-525` | §3.4.7 | 212; notifies at 333, 412, 418, 424, 440, 460, 518, 525 | correct, but incomplete: completion is `live/mod.rs:910` (A14) |
| 22 | `vendor/rqbit/librqbit/src/torrent_state/initializing.rs:113-114` no fastresume | D3, P6-06 | the two comment lines | correct |
| 23 | `vendor/rqbit/librqbit/src/session.rs:140` `ratelimits`, `limits.rs:85-91` | §6.2 row 5 | both | correct |
| 24 | `vendor/rqbit/librqbit-core/Cargo.toml:30` / `librqbit/Cargo.toml:33` sha1 conflict | D11 | `default = ["sha1-crypto-hash"]` 30; `rust-tls = [... "librqbit-core/sha1-ring"]` 33 | correct |
| 25 | `plugin/oliver.torrents/Service.qml:121-126` reconcile timer | D1 | the 60 s `Timer` at 121–126 | correct |
| 26 | `plugin/oliver.torrents/Panel.qml:18` dim 1.55, `:179` `q` stops, `:410` literal 400 | §1.5, Appendix B, §4.13 | all three | correct |
| 27 | `Commons/Style.qml:236-245` spacing, `:329-336` type, `:363-367` motion | §4.2.1 | 237–245 (236 is `hairline`), 329–336, 363–367 | correct (one line early on the first) |
| 28 | `Ui/ConfirmDialog.qml:12` `selectedIndex: 1`, `:122` `onEntered` | §4.9 | both | correct |
| 29 | `Ui/PanelKeyCatcher.qml:86-92` Enter/Space, `:39,42` reorder | §4.12, Appendix B | 86–92 Enter then Space; 39, 42 | correct |
| 30 | `panels/network/Panel.qml:1109`, `bluetooth:670`, `tailscale:421`, `agents:972` width 380 | §4.2.2 | the first three; `agents` is `plugins/agents/Panel.qml:972` | one path wrong (A18) |
| 31 | `shell.qml:1161-1172`, `1356-1372`; `PluginRegistry.qml:148-163` | A3 | all three | correct |
| 32 | `~/.config/omarchy/extensions/omarchy-menu.jsonc:36` qbittorrent gate, `:38` 󰦝 | D18, §4.2.4 | line 36 `systemctl --user is-active --quiet qbittorrent-nox`; line 38 the Proton entry with 󰦝 | correct |
| 33 | `ftui-core-0.6.0/src/terminal_session.rs:107,121,151,1078`; `ftui-runtime-0.6.0/src/program.rs:111-117,5484-5490` | D2, P0-08 | `SIGNAL_SHUTDOWN_GRACE` 107, `EXPLICIT_TEARDOWN_STARTED` 121/151/1078; wrappers 111/117; `clear_termination_signal()` in `complete_lifecycle` 5484–5490 | correct |

Also checked: `notify-send --help` lists `-A/--action` ("Implies --wait … The name of the action
is output to stdout"); `systemd-inhibit`, `grim`, `wl-paste`, `wl-copy`, `wtype`, `hyprctl`,
`omarchy-shell`, `omarchy-launch-or-focus-tui`, `omarchy-theme-set`, `omarchy-restart-shell`,
`qs`, `inotifywait` are all on `PATH`; `omarchy-shell shell summon <id> '<json>'` is the three-
argument form the script accepts (`/usr/share/omarchy/bin/omarchy-shell:58`); `Quickshell.Io`
exports `Socket`, `SplitParser`, `DataStreamParser`, `FileView`; `Quickshell.clipboardText`
exists; `PluginRegistry.qml:716-727` runs `inotifywait -m -r` (the §10.4 symlink question
stands); `SUPER + CTRL + T` is "Activity" (`utilities.lua:108`); the plugin symlink
`~/.config/omarchy/plugins/oliver.torrents → …/fbittorrent/plugin/oliver.torrents` exists;
`shows.json` and `engine/torrents.json` are 0644 (D10 understates: the ledger is too; P0-14's
"check `engine/*`" covers it).

### F.2 Eight numeric claims, recomputed

| # | Claim | Recomputed | Verdict |
|---|---|---|---|
| 1 | Dashboard chrome 636 / 246 / 16; fixed columns 748; 532 without ADDED, RATIO, ↑; NAME 216 at 1600, 232–431, 322–621, 352–551; a docked pane needs ≥ 1378; the pane costs 390 | 200+360+28+48 = 636; 200+14+32 = 246; 26+110+76+120+80+80+64+56+56+80 = 748; 748−216 = 532; 1600−636−748 = 216; 1400/1599−636−532 = 232/431; 1100/1399−246−532 = 322/621; 900/1099−16−532 = 352/551; 636+532+210 = 1378; 360+14+16 = 390 | all correct |
| 2 | Owner's theme: dim rung 1 `#b6b6b6` at 10.4:1; `#505050` 2.6:1; `muted #7a7a7a` 4.9:1; accent `#8d8d8d` 6.3:1 on black and 3.3:1 against white, channel difference 114; red/green difference 18 | WCAG over `colors.toml` (`fg #ffffff`, `bg #000000`): 10.35, 2.60, 4.89, 6.33, 3.32, 255−141 = 114, 182−164 = 18 | all correct |
| 3 | Over the 23 installed themes: `muted` ≥ 4.5:1 on 3; rung 1 fails on `everforest` (3.8), `gruvbox` and `tokyo-night` (4.3); accent < 4.5:1 on `catppuccin-latte` (4.3), `miasma` (3.9), `rose-pine` (3.1); `rose-pine` foreground 6.7:1 and every other theme ≥ 7:1; `catppuccin-latte` dim 10.2:1; `kanagawa` accent equals foreground; `tokyo-night` accent 1.19 against foreground with channel difference 47 | script over `/usr/share/omarchy/themes/*/colors.toml` (`r4/contrast.py`): 3 (ethereal, starship, vantablack); 3.8 / 4.3 / 4.3 and rung 2 passes all three (4.9, 5.2, 5.2); 4.3 / 3.9 / 3.1; 6.7 and the next lowest 7.1 (catppuccin-latte); 10.2; ratio 1.00, difference 0; 1.19 and 47 | all correct (the owner's theme is `vantablack`'s palette) |
| 4 | §9.4 weights: M1a/M1 31; sizes L 13, M 44, S 42 of 99 items; 107 nodes | path sum 31 (A1); 13+44+42 = 99; 99+8 = 107 | correct; §9.2's "26" is the stale one (A1) |
| 5 | `src/tui/mod.rs` 3,504 lines, `src/engine.rs` 2,694, `src/main.rs` 1,300 | `wc -l` | correct |
| 6 | Working tree: 19 modified files +6363/−426; about 3,575 untracked files; `vendor/` 60 MB (asupersync 57, rqbit 3.3, nova3 64 KB); `docs/validation/` 279 MB, 2,197 files, six dated directories; asupersync `src/` 56 MB in 959 `.rs` files; the binary 21,555,840 bytes; the ratings file 28.7 MB | `git diff --shortstat`: 19 files, +6363/−426; `git status --untracked-files=all`: 3,584; `du`: 60M, 57M, 3.3M, 64K, 279M; `find`: 2,197; six directories 10-09…10-14; 56M, 959; `ls -l`: 21,555,840; 30,052,145 bytes = 28.66 MiB | correct (3,584 vs "about 3,575": fine) |
| 7 | `Pieces.have`: 100,000 pieces are 16.7 KB; a 200,000-piece frame is under 64 KiB; a 1 MiB line fits a 256-item chunk of ≤ 2 KiB torrents; a frozen shell fills about 480 of 512 frames | 12,500 bytes → 16,668 base64 chars; 25,000 → 33,336 (< 65,536); 256 × 2 KiB = 512 KiB; 8 topics × 60 s = 480 | all correct |
| 8 | systemd defaults "5 starts per 10 s"; `Qt.darker(#ffffff, 1.4)` = `#b6b6b6`; "Downloading" ≈ 86 px at 13 px subtitle | `systemctl --user show`: `DefaultStartLimitBurst=5`, `DefaultStartLimitIntervalUSec=10s`; 255 × 100 ÷ 140 = 182 = `0xb6`; JetBrains Mono advance 0.6 em × 13 px × 11 = 85.8 | all correct |

---

## G. Owner questions (only the genuinely theirs)

| # | Question | Why it is yours |
|---|---|---|
| G1 | Q3, third clause (C5): the Asupersync rider forbids making the Software or a derivative "available to" OpenAI/Anthropic and their agents, and names "analyzing" and "testing" as use. Every phase of this plan has Claude Code or Codex read `vendor/asupersync/` and build the binary that links it. Do you accept that reading of the rider for the project, independent of where the patch is hosted? | A licence risk only the owner can take; the plan correctly draws no conclusion |
| G2 | Q1 restated with B4 attached: if "keep running while there is work, then stop after 5 idle minutes" is right, may the `last-ui` and `explicit` policies go, with `idle_exit_secs = 0` meaning "never"? | Removing two settings is a product decision |
| G3 | The popup's key-hint row (C1): hidden until the first key press, or always visible as round 3 specified? The first is what the Wi-Fi panel looks like; the second is what a keyboard user sees at once | The owner's "mistaken for first-party" bar is theirs to weigh against discoverability |
| G4 | M1a's `undo` toast: with A5 the daemon posts the actioned "Added" toast only for browser clicks (`origin: handler`); `fbittorrent add` from a terminal keeps a plain toast with no Undo. Fine, or should a TTY `add` also get the actioned toast (and block until it closes)? | The CLI is the owner's second daily surface |

---

## H. Hunk verification

`r4/verify_hunks.py` parses every ` ```diff ` block in this file, strips the leading `-` from
every removed line (ignoring `---`/`+++` headers) and the leading space from every context line,
and checks each against `PLAN.md` as an exact line; it also reports how often each line occurs
(a line that occurs more than once would need more context to apply). Output:

```
$ python3 -I r4/verify_hunks.py docs/plan-2026-10-18/reviews/round-4-fresh-read.md docs/plan-2026-10-18/PLAN.md
hunks: 73; removed lines: 149; context lines: 6; added lines: 298
removed lines not found verbatim in PLAN.md: 0
context lines not found verbatim in PLAN.md: 0
quoted lines that occur more than once in PLAN.md (need their neighbours to apply): 0
```

Every `-` line and every context line of the 73 hunks occurs exactly once in the round-3 `PLAN.md`,
so each hunk applies by content without a line number. Scratch files: `r4/item-P1-06.md`,
`r4/item-P1-08.md`, `r4/item-P3-09.md`, `r4/item-P4-04.md` (the probe inputs), `r4/contrast.py`
(the theme recomputation of F.2 row 3), `r4/verify_hunks.py` (this script).
