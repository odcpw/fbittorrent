# Round 3 review: whole-plan coherence after the round-2 merge

Reviewer: independent (did not write the plan, did not see rounds 0-2 except through
`review-log.md`). Inputs read in full: `PLAN.md` (5,075 lines), `review-log.md`, `README.md`,
`reality-check.md`, `omatorrent-research.md`, the cited sections of `frontend-options.md`, both
round-2 reviews, `evidence/`, `screens/tui-real.png`, `screens/30-bar-right-widgets.png`. Code and
shell files checked where a claim depended on them: `src/{main,agent,engine,ipc,network_app,
config,backend}.rs`, `src/tui/{mod,theme}.rs`, `plugin/oliver.torrents/*`,
`/usr/share/omarchy/shell/{Ui/ConfirmDialog,Ui/PanelKeyCatcher,Ui/PanelHero,Ui/PanelSectionHeader,
Commons/Style,Commons/Color}.qml`, `shell.qml`, `plugins/panels/network/Panel.qml`, the owner's
`shell.json`, `omarchy-menu.jsonc`, `~/.config/omarchy/plugins/`, `vendor/asupersync/` (`du`),
`vendor/rqbit/librqbit/src/{torrent_state/mod,limits,session}.rs`. Lenses: `planning-workflow`,
`just-say-no-to-process-porn-and-ceremony`, `simplify-and-refactor-code-isomorphically`, `ux-audit`.

Hunks below quote PLAN.md lines verbatim; `--- PLAN.md (line N)` gives the line of the first
removed line so an integrator can apply by content with the line as a hint. Section E is the
output of the script that checked every removed and context line against PLAN.md.

## Verdict

The plan is coherent in architecture and almost everywhere in detail: the daemon, API, two UIs and
CLI tell one story, the DAG is acyclic, and most rules have exactly one home. The round-2 merge
left a small number of real contradictions that a fresh agent would hit on day one and would have
to resolve by guessing: the engine slot becomes `Running` at two different moments (§3.2.4 versus
§3.2.7); the "dim" colour is defined by three incompatible ladders (§4.2.1, §5.5.1, DL-23, DL-31)
and then tested for hex equality across them; "idempotent in Appendix A" refers to a marking
Appendix A does not have, with two different idempotent lists; the harness every P1 item's tests
run on (`daemon.py`, P1-07) depends on the items it tests; `ensure_daemon()` is specified against a
socket that does not exist yet when its item lands; and the keymap breaks its own "same letters
everywhere" rule on `q` (closes the TUI and the dashboard, stops the daemon from the popup). A
fresh agent can start P0-01 tomorrow; it cannot finish P1-03/P1-04 without the fixes in A1, A3,
A10 and A20, and cannot write P2-08/P4-05 without A2.

Ceremony is modest for a plan of this size and the plan already removed one item for it; what
remains is mostly duplicated test layers and gates that measure nothing the owner will act on:
a second process-level test harness in Rust next to `daemon.py`, legacy-binary compatibility tests
for a binary that is replaced in one `mv`, a 1-pixel automated comparison against a third-party
panel's screenshot, CI jobs that lint the plan document itself, a minor protocol version nobody
branches on, a unit test asserting code matches a planning appendix, and a conditional marketplace
item for a marketplace the owner has excluded. Section B cuts them without removing a feature.

Top five risks after this round: (1) the critical path to the first user-visible change is 35
S-units of serial work (four L items in a row) and the first demo is at the end of P2; P1's end
is demo-able and should be a milestone (C2); (2) the Asupersync decision (Q3) blocks P0-04 and
therefore everything, and §8.2's "option B" rests on a wrong premise (`src/` alone is 56 MB), so
the owner must answer before the first commit (A23, C1); (3) the dim-ladder contradiction would
produce a QML popup and a TUI that disagree on the owner's own theme and a test that can never
pass (A2); (4) the P1-07 dependency inversion means P1-04 and P1-06 cannot be accepted when they
land (A10); (5) UI contract files (`states.json`, `expected.json`, `keymap.json`) have no
specified shape, so the first author of each invents one and the second must reverse-engineer it
(A12).

## Section A. Defects

### A1. The engine slot becomes `Running` at two different moments

Locations: §3.2.4 (lines 699-704, DL-49, P0-09) say the slot is `Running` as soon as the librqbit
session exists and restores run in background tasks. §3.2.7's transition table (line 775) says
`Starting → Running` fires when "`engine::start` returns (after restores, `src/engine.rs:455-471`)",
and the diagram (line 754) labels `Starting` with "checks run". Round 2 applied B4 to §3.2.4 and
P0-09 but not to §3.2.7. A fresh agent implementing P1-03 from §3.2.7 (the section P1-03 cites)
would keep today's serial restore, which is the behaviour DL-49 removes. Evidence:
`src/engine.rs:453-466` awaits every restore in series today, as §3.2.4 says.

```diff
--- PLAN.md (line 754)
-   │ Absent │                                         │ Starting │ slot: Starting (ledger read, checks run)
+   │ Absent │                                         │ Starting │ slot: Starting (ledger read; no session yet)
```

```diff
--- PLAN.md (line 775)
-| Starting → Running | `engine::start` returns (after restores, `src/engine.rs:455-471`) | `engine.changed {epoch: 1, state: "running"}`; fresh snapshots |
+| Starting → Running | the librqbit session exists (DL-49, §3.2.4); saved records are restored by background tasks afterwards and show as `checking` or `metadata` meanwhile | `engine.changed {epoch: 1, state: "running"}`; fresh snapshots |
```

### A2. Three incompatible "dim" ladders, and a test that requires them to agree to the hex

Locations and what each says:

- §4.2.1 token row (line 1484): `Qt.darker(fg, 1.4)` → `Color.muted` → blend 0.75 → blend 0.85 → `fg`.
- §4.2.1 rule bullet (line 1509): "`muted` is reported for information and never used for text."
- §5.5.1 dim row (line 2287): `muted` first → `dark_foreground` → blend.
- §5.7 rows 3 and 16 (lines 2430, 2443): "the contrast rule of §5.5.1 (`muted` first)".
- DL-23 (line 4290): QML 1.4; TUI `muted`/`dark_foreground`/blend.
- DL-31 (line 4298): "one ladder (1.4 → `muted` → blends) in QML and TUI; `muted` is never a text colour" (self-contradictory).
- P2-08 Do (lines 3467-3468): `contrast.py` computes ratios for "dim (1.4)".
- P2-08 and P4-05 accept: QML and TUI must return "the same hex as `palette.rs` for every fixture theme" / "the dim hex per fixture theme equals `expected.json`".

On the owner's theme the §4.2.1 ladder yields `#b6b6b6` (1.4 of white, 10.6:1) and the §5.5.1
ladder yields `#7a7a7a` (`muted`, 4.9:1): the two surfaces disagree on the owner's own theme and
the hex-equality test fails by construction. Two further guesses a fresh agent must make: what
`Qt.darker` does exactly (Qt divides the HSV value by the factor; the TUI must reproduce that to
match a hex), and what "blend toward background at 0.75" means (which weight, which colour space,
which rounding). Fix: one ladder, defined once in §4.2.1 with the formulas and a ±1 rounding
tolerance; drop `muted` and `dark_foreground` from it (the plan's own rule says `muted` is never
text; `dark_foreground` is D16); every other location becomes a pointer.

```diff
--- PLAN.md (line 1484)
-| Dim text | `Model.dim(foreground, background)`: the first of `Qt.darker(foreground, 1.4)` (the shell's convention), `Color.muted`, and a blend of `foreground` toward `background` at 0.75 then 0.85 that reaches 4.5:1; `foreground` itself as the last resort | theme | `Ui/PanelHero.qml:23`, `Ui/PanelSectionHeader.qml:20`; the same ladder as the TUI's §5.5.1, one fixture (`contracts/ui/themes/`) |
+| Dim text | `Model.dim(foreground, background)` in QML and `palette::dim(fg, bg)` in the TUI: **one ladder, defined here and nowhere else.** Rung 1: `Qt.darker(foreground, 1.4)`, which is Qt's HSV darken (value × 100 ÷ 140, hue and saturation unchanged; the TUI reimplements exactly that). Rung 2: `mix(foreground, background, 0.75)`, per channel in 8-bit sRGB, `round(0.75·fg + 0.25·bg)`. Rung 3: the same mix at 0.85. Rung 4: `foreground`. The first rung whose WCAG contrast against `background` reaches 4.5:1 wins. `muted` and `dark_foreground` are never rungs (`dark_foreground` is 2.6:1 on the owner's theme, D16; `muted` reaches 4.5:1 on 3 of 23 installed themes). `contrast.py` computes the reference hex per fixture theme into `contracts/ui/themes/expected.json`; QML and TUI must match it within ±1 per channel (rounding) | theme | `Ui/PanelHero.qml:23`, `Ui/PanelSectionHeader.qml:20` (the 1.4 convention); one fixture (`contracts/ui/themes/`) |
```

```diff
--- PLAN.md (line 2287)
-| dim (headers, hints, secondary) | `muted`, else `dark_foreground`, else a blend | **must reach 4.5:1 against `background`**; try `muted` first (the owner's `#7a7a7a` gives about 4.9:1), then `dark_foreground` (`#505050` gives 2.6:1 and fails), then blend `foreground` toward `background` until the ratio is at least 4.5:1. Fixes D16; today `theme.rs:66-67` prefers `dark_foreground`. |
+| dim (headers, hints, secondary) | `foreground` and `background` through the ladder of §4.2.1 (the only definition) | **must reach 4.5:1 against `background`**, which the ladder guarantees; on the owner's theme rung 1 gives `#b6b6b6` (10.6:1), the same hex the shell's own panels use. Fixes D16: today `theme.rs:66-67` loads `muted` and then overwrites it with `dark_foreground` (`#505050`, 2.6:1); neither key is used for text any more. |
```

```diff
--- PLAN.md (line 2430)
-| 3 | Dimmed text `#505050` on black (about 2.6:1) | The contrast rule of §5.5.1 (`muted` first) | P4-05 |
+| 3 | Dimmed text `#505050` on black (about 2.6:1) | The dim ladder of §4.2.1, applied by `palette.rs` (§5.5.1) | P4-05 |
```

```diff
--- PLAN.md (line 2443)
-| 16 | Dim prefers `dark_foreground` over `muted` (D16) | §5.5.1 | P4-05 |
+| 16 | Dim prefers `dark_foreground` over `muted` (D16) | §4.2.1 ladder via §5.5.1; neither key is a text colour | P4-05 |
```

```diff
--- PLAN.md (line 3467)
-  optional theme switch and restore); `tests/contrast.py` computing WCAG ratios for foreground,
-  dim (1.4) and `muted` against `background` for every `/usr/share/omarchy/themes/*/colors.toml`.
+  optional theme switch and restore); `tests/contrast.py` computing, for every
+  `/usr/share/omarchy/themes/*/colors.toml` and the owner's, the §4.2.1 ladder (which rung wins and
+  its hex) and the WCAG ratios of `foreground`, the computed dim and `accent` against `background`,
+  and writing the reference `contracts/ui/themes/expected.json` (A12 gives its shape).
```

```diff
--- PLAN.md (line 4290)
-| DL-23 (new) | Dim text is `Qt.darker(fg, 1.4)` in QML and the first of `muted`/`dark_foreground`/blend that reaches 4.5:1 in the TUI | §4.2.1, §5.5.1 |
+| DL-23 (new) | Superseded by DL-31 (round 3): one ladder for both surfaces. | §4.2.1 |
```

```diff
--- PLAN.md (line 4298)
-| DL-31 (round 2) | Dim text is computed by one ladder (1.4 → `muted` → blends) in QML and TUI from one theme fixture (`contracts/ui/themes/`); `muted` is never a text colour; accent falls back to bold foreground below 4.5:1; the TUI's text gate is 4.5:1, not 7:1 | §4.2.1, §5.5.1, P2-08, P4-05 |
+| DL-31 (round 2, corrected round 3) | Dim text is computed by one ladder (1.4 → mix 0.75 → mix 0.85 → foreground) in QML and TUI, defined once in §4.2.1, from one theme fixture (`contracts/ui/themes/`); `muted` and `dark_foreground` are never text colours; accent falls back to bold foreground below 4.5:1 (§5.5.1); the TUI's text gate is 4.5:1, not 7:1 | §4.2.1, §5.5.1, P2-08, P4-05 |
```

### A3. "Idempotent in Appendix A" points at a marking Appendix A does not have, and two lists disagree

Locations: §3.2.4 step 6 (lines 692-695) lists `start, stop, set_seed, set_files,
set_upload_limit, add, commit, discard, settings` as "marked idempotent in Appendix A"; §3.2.11
(lines 880-881) lists `start, stop, set_seed, set_files`; P1-18 says `api --list` prints an
`idempotent` flag from `api::ops::TABLE`. Appendix A's op tables have `Mutating` and `Since`
columns and no idempotency column. A fresh agent writing `api::ops::TABLE` has to decide, per
operation, whether a resend after a restart is safe, and the two lists give different answers for
`add`, `commit`, `discard` and `set_upload_limit`. Fix: one rule in A.1, both sections point at it.

```diff
--- PLAN.md (line 4398)
 - Lines: client → server ≤ 65,536 bytes; server → client ≤ 1,048,576 bytes.
+- Idempotent operations (safe to resend with the same `ref` after a mid-request loss, §3.2.4, or
+  against a new instance after a crash, §3.2.11): `torrents.start`, `torrents.stop`,
+  `torrents.set_seed`, `torrents.set_files`, `torrents.set_upload_limit`, `torrents.add`,
+  `torrents.commit`, `torrents.discard`, `torrents.force_start`, `app.settings.set`,
+  `transfer.set_limits`, `transfer.set_alt`, `search.set_provider`, `rss.set_read`,
+  `rss.mark_feed_read`, `shows.set_quality`, `shows.set_read`, `shows.mark_show_read`. Every other
+  mutating operation (`torrents.remove`, `torrents.recheck`, `torrents.move`, `torrents.rename`,
+  `torrents.queue_move`, `rss.add`, `rss.remove`, `shows.follow`, `shows.unfollow`,
+  `shows.undo_read`, `search.install`, the `*.grab` family, `app.shutdown`, `app.restart`) is not,
+  and a client never resends it on its own. `api::ops::TABLE` carries this flag and `api --list`
+  prints it (P1-18); this list is the source.
```

```diff
--- PLAN.md (line 692)
    request was sent and before its response: for a read, or for a mutation whose op is marked
-   idempotent in Appendix A (`start`, `stop`, `set_seed`, `set_files`, `set_upload_limit`, `add`,
-   `commit`, `discard`, settings), re-run steps 1-4 once and resend with the **same `ref`**; for
+   idempotent in A.1, re-run steps 1-4 once and resend with the **same `ref`**; for
    `remove`, `recheck`, `move` exit 8 (`timeout`) with the hint `fbittorrent list --json` and the
```

```diff
--- PLAN.md (line 880)
   the double-apply the ring exists to prevent, so the new daemon applies it as a fresh request
-  only for idempotent operations (start, stop, set_seed, set_files) and refuses `remove` with
+  only for the idempotent operations of A.1 and refuses every other mutation with
   `invalid_state` ("daemon restarted since this request; check the list").
```

### A4. Esc's last layer: §4.12 closes the window, Appendix B and DL-34 say it never does

Locations: §4.12 (lines 2053-2054) "Esc steps back one layer at a time: dialog, then filter text,
then details, then the window"; Appendix B (line 4672) and DL-34 (line 4301) say Esc at the top
level "does nothing (a long-lived window must not vanish on a reflex key)"; P3-09's acceptance
tests exactly that (`q_closes_esc_does_not`). The §4.12 sentence is the pre-round-2 text.

```diff
--- PLAN.md (line 2053)
-- Esc steps back one layer at a time: dialog, then filter text, then details, then the window
-  (ucmz851's layered Esc [OR §5]).
+- Esc steps back one layer at a time: dialog, then filter text, then details, then the selection;
+  at the top level it does nothing, and `q` closes the window (Appendix B, DL-34: a long-lived
+  window must not vanish on a reflex key). This is ucmz851's layered Esc [OR §5] without its last
+  step.
```

### A5. Who may start the daemon from the plugin: three answers

Locations: §4.4 rule 10 (lines 1760-1761) "The service never starts the daemon"; §4.4 actions
(lines 1776-1777) "`startDaemon` (dashboard only)"; §4.5 (line 1820) "With `showWhenStopped`, a
left click on the dimmed icon starts the daemon (`startDaemon`)"; §4.11 popup column "Enter Start";
§3.2.3 "Bar service connection: never". The rule the plan means is "never implicitly"; the three
explicit entry points (dashboard [Start], popup Enter on "not running", dimmed-icon click) all go
through one action. Say so once.

```diff
--- PLAN.md (line 1760)
 10. When the socket cannot connect, `online = false` and the backoff caps at 10 s. The service
-    never starts the daemon.
+    never starts the daemon on its own; only the explicit `startDaemon` action (below) does, and
+    only when a person pressed or clicked Start.
```

```diff
--- PLAN.md (line 1776)
-`ref = "qs-" + startMillis + "-" + n`. `startDaemon` (dashboard only) runs `fbittorrent up` once
-through `Util.execArgv`; it is a one-shot, not a poller.
+`ref = "qs-" + startMillis + "-" + n`. `startDaemon` runs `fbittorrent up` once through
+`Util.execArgv` and is the only way the plugin ever starts the daemon: the dashboard's [Start],
+the popup's Enter on its "not running" state (§4.11) and a click on the dimmed icon under
+`showWhenStopped` (§4.5) call it; nothing calls it without a person's action. It is a one-shot,
+not a poller.
```

### A6. "Same letters everywhere" is broken by `q`, and the exception count is stated three ways

Locations: §2.2 principle 7 (line 410) "Appendix B lists the two exceptions"; §4.12 (line 2064)
"One exception remains"; Appendix B (line 4708) "One exception"; DL-33 "the only key exception".
Meanwhile Appendix B's own rows (lines 4672-4673) make `q` close the TUI and the dashboard but
**quit the daemon** from the popup (confirming only when transfers are active, so with nothing
active one reflex `q` kills the daemon from a glance surface), and `w` opens Wikipedia in the
dashboard and the TUI but the dashboard window from the popup. UX audit (heuristic 4, consistency;
heuristic 3, control): the TUI's `q` is the owner's muscle memory for "close this, leave downloads
running"; the popup turning that into "stop fbittorrent" is the one place a wrong letter costs
something. Fix: `q` closes the popup like Esc; `Q` and the power icon quit; declare the two real
exceptions (Enter, `w`) in one place and point there from the other three.

```diff
--- PLAN.md (line 4672)
-| Close this UI | `q`, Ctrl+C (no longer stops downloads) | `q`, or close the window; Esc at the top level clears the filter and the selection and then does nothing (a long-lived window must not vanish on a reflex key) | Esc |
-| Quit fbittorrent | `Q` (confirms when active) | `Q`, Ctrl+Q | `q` or the power icon (confirms when active) |
-| Torrent: primary action | Enter opens details (Overview first); double-click too | Enter opens details; double-click too | Enter opens the file (the one exception) |
+| Close this UI | `q`, Ctrl+C (no longer stops downloads) | `q`, or close the window; Esc at the top level clears the filter and the selection and then does nothing (a long-lived window must not vanish on a reflex key) | Esc, `q` |
+| Quit fbittorrent | `Q` (confirms when active) | `Q`, Ctrl+Q | `Q` or the power icon (confirms when active) |
+| Torrent: primary action | Enter opens details (Overview first); double-click too | Enter opens details; double-click too | Enter opens the file (exception 1) |
```

```diff
--- PLAN.md (line 4708)
-One exception to "same letters everywhere", kept for muscle memory: the popup's Enter opens the
-file (today's behaviour, `Panel.qml:72-76`); the dashboard's and the TUI's Enter open details.
+Two exceptions to "same letters everywhere", both in the popup and both listed only here: (1) Enter
+opens the file (today's behaviour, `Panel.qml:72-76`) where the dashboard's and the TUI's Enter
+open details; (2) `w` opens the dashboard window where the dashboard's and the TUI's `w` open
+Wikipedia (the popup has no IMDb card, so the letter is free there). `q` is not an exception: it
+closes the surface it is pressed in, everywhere, and `Q` quits everywhere (today's popup treats
+`q` and `Q` alike and stops the daemon, `Panel.qml:179`; that changes in P2-03).
```

```diff
--- PLAN.md (line 410)
-   unless the platform forces otherwise (Appendix B lists the two exceptions).
+   unless the platform forces otherwise (Appendix B lists the exceptions; there are two).
```

```diff
--- PLAN.md (line 2064)
-  first; this is how that bug is fixed instead of documented. One exception remains to "same
-  letters everywhere": the popup's Enter opens the file (as today, `Panel.qml:72-76`) where the
-  dashboard's Enter opens details. It is listed in Appendix B.
+  first; this is how that bug is fixed instead of documented. The exceptions to "same letters
+  everywhere" are listed once, at the end of Appendix B.
```

```diff
--- PLAN.md (line 1855)
-- **Quit** (`q` or the power icon) asks only when transfers are active: "Quit fbittorrent? 2
+- **Quit** (`Q` or the power icon; `q` closes the popup, as it closes the TUI and the dashboard)
+  asks only when transfers are active: "Quit fbittorrent? 2
```

```diff
--- PLAN.md (line 4300)
-| DL-33 (round 2) | Space pauses and resumes in every surface (`returnRequested` tells Enter from Space); the popup's Enter opening the file is the only key exception | §4.12, Appendix B |
+| DL-33 (round 2, extended round 3) | Space pauses and resumes in every surface (`returnRequested` tells Enter from Space); `q` closes the surface it is pressed in and `Q` quits, everywhere; the popup's Enter (opens the file) and `w` (opens the window) are the two key exceptions, listed in Appendix B | §4.12, Appendix B |
```

### A7. Appendix C's dashboard footers show `s providers`, which DL-34 settled as `P`

Locations: Appendix C lines 4792 and 4808 ("b lists  s providers", "b browse  s providers") versus
Appendix B (line 4696: `b` / `P` / `O`; `s` is sort everywhere) and DL-34. The mockups are what a
plugin author copies the hint row from. Same cell width, so `mockwidth.py` stays clean.

```diff
--- PLAN.md (line 4792)
-│ Enter add  i IMDb  v rows  b lists  s providers  / search  Esc back                                           │
+│ Enter add  i IMDb  v rows  b lists  P providers  / search  Esc back                                           │
```

```diff
--- PLAN.md (line 4808)
-│ Enter add  f follow  i IMDb  O source  b browse  s providers  Esc back                                        │
+│ Enter add  f follow  i IMDb  O source  b browse  P providers  Esc back                                        │
```

### A8. The plugin gives up on a request before the server does

Locations: §4.4 rule 6 (line 1748) "the pending map times out after 10 s (95 s for
`torrents.add`)"; §3.4.10 "Default op timeout 15 s; `torrents.add` 95 s". A client timeout shorter
than the server's means the server's answer can arrive after the row already says "no answer", and
`torrents.add` has no margin at all. The rule also says "non-destructive mutations are resent",
which is not the A.1 vocabulary.

```diff
--- PLAN.md (line 1748)
-6. Requests carry increasing ids. The pending map times out after 10 s (95 s for `torrents.add`);
-   a timeout marks that action "no answer". After a reconnect, pending non-destructive mutations
-   are resent with the same `ref`; removes are never resent.
+6. Requests carry increasing ids. The pending map times out after 20 s (100 s for `torrents.add`),
+   always longer than the server's own op timeouts (15 s and 95 s, §3.4.10), so an answer can never
+   arrive after the client has given up on it; a timeout marks that action "no answer". After a
+   reconnect, pending idempotent mutations (A.1) are resent with the same `ref`; nothing else is.
```

### A9. `log` and `events` are listed as "read the ledger" commands but have nothing to read

Locations: §3.2.3 (line 665) puts `log` and `events` in the row that "reads the ledger; reports
`running: false`", then says only `events --follow` exits 4. Appendix F gives `log` "0, 4" and
`events` "0, 2, 4"; P1-18 says `events` exits 4 when the daemon is down, with or without
`--follow`. The log ring and the event ring live only in the daemon.

```diff
--- PLAN.md (line 665)
-| `status`, `list`, `show`, `triage`, `log`, `events`, `config`, `capabilities`, `robot-docs` | no | Read the ledger; report `running: false`; progress is absent (unknown). `events --follow` exits 4 with the hint `fbittorrent up`. |
+| `status`, `list`, `show`, `triage`, `config`, `capabilities`, `robot-docs` | no | Read the ledger; report `running: false`; progress is absent (unknown). |
+| `log`, `events` | no | Exit 4 with the hint `fbittorrent up`, with or without `--follow`: the log ring and the event ring exist only inside the daemon (journald keeps the long log history: the hint also prints `journalctl --user -u fbittorrent`). |
```

### A10. The daemon harness depends on the items whose tests run on it, and P1-04 is specified against a socket that does not exist yet

Locations: P1-07 `daemon.py` *Depends on:* P1-04, P1-06, P0-15 (line 3191), while P1-03's tests
are in a separate `tests/daemon_core.rs`, and P1-04's and P1-06's *Tests* are `daemon.py::…` cases
(lines 3137-3141, 3167-3173). So P1-04 and P1-06 cannot meet "done means the acceptance criteria
pass" (§9.1) until P1-07 lands after them. Second: §3.2.4 step 1 has `ensure_daemon()` send
`hello` to `api.sock`, and §3.2.4 says `READY=1` is sent "once the API socket listens"; both the
socket and `hello` arrive with P1-06, but P1-03 (which sends `READY=1`) and P1-04 (which implements
`ensure_daemon()`) depend only on each other. A fresh agent on P1-04 has to invent an interim
probe. Fix: P1-07 becomes the harness skeleton (temp XDG, shims, fixture server, case runner,
logging), depending on P0-15 only; P1-03 depends on it and adds the first daemon cases; the
daemon-specific helpers (peer pair, NDJSON client, `--serve-fixture`) arrive with the items that
need them; `ensure_daemon()` probes `engine.sock` with `app/version` until P1-11 switches it to
`hello`; `READY=1` is tied to "the daemon's socket", which is `engine.sock` first. The Rust twin
harness goes (B1). The §9.4 row for P1-03 is in C3 (one combined edit).

```diff
--- PLAN.md (line 699)
-`READY=1` is sent once the API socket listens and the engine slot exists in `Starting`, not after
+`READY=1` is sent once the daemon's socket listens (`engine.sock` from P1-03; `api.sock` as well
+from P1-06) and the engine slot exists in `Starting`, not after
```

```diff
--- PLAN.md (line 3126)
-- *Do:* §3.2.2 to §3.2.4 and §3.2.14: `setup` writes the unit; `ensure_daemon()`; `up` (start only;
+- *Do:* §3.2.2 to §3.2.4 and §3.2.14: `setup` writes the unit; `ensure_daemon()` (its liveness probe is `app/version` on `engine.sock` until P1-11 switches it to `hello` on `api.sock`); `up` (start only;
```

```diff
--- PLAN.md (line 3177)
-- *Do:* `agent_ergonomics_audit/audit/regression_tests/daemon.py` on `fbtlog.py`: temporary XDG
-  directories, a local HTTP fixture server, the local magnet fixture from `native.py`, a local
-  seed/leech peer pair (a second `fbittorrent serve` in another temp dir seeding the fixture;
-  a `--throttle` option to cap its upload for P5-05), the daemon in the foreground or through the
-  unit (`--systemd`), an NDJSON client helper, `PATH` shims, a `--serve-fixture <state>` mode that
-  leaves a daemon running with fixture torrents in named states for the visual harness (P2-08),
-  and the case runner (`daemon.py --case name`, `--list`). Cases for P1-03, P1-04 and P1-06 now;
-  later items add theirs.
+- *Do:* `agent_ergonomics_audit/audit/regression_tests/daemon.py` on `fbtlog.py`, as a skeleton
+  that needs no daemon to exist: temporary XDG directories, a local HTTP fixture server, the local
+  magnet fixture from `native.py`, `PATH` shims, a process spawner that captures stderr into
+  `<case>.daemon.log`, the case runner (`daemon.py --case name`, `--list`) and the `--systemd` /
+  `--no-systemd` switch (a throwaway unit name derived from the temp dir). Later items add the
+  helpers they need and their cases: P1-03 the first daemon cases (A10), P1-06 the NDJSON client
+  helper, P1-07's own second half is `--serve-fixture <state>` (a daemon left running with fixture
+  torrents in named states for the visual harness, P2-08) and the local seed/leech peer pair (a
+  second `fbittorrent serve` in another temp dir seeding the fixture, with `--throttle` for P5-05),
+  which land as a follow-up commit once P1-06 exists.
```

```diff
--- PLAN.md (line 3191)
-- *Depends on:* P1-04, P1-06, P0-15.
+- *Depends on:* P0-15 (the skeleton); its peer-pair and `--serve-fixture` half depends on P1-06.
```

```diff
--- PLAN.md (line 3119)
-- *Tests:* `tests/daemon_core.rs` (Rust integration, spawns the binary): `ready_notify`,
-  `parked_start`, `sigterm_drains`, `panic_exits_70`, `second_serve_refused`, each logging through
-  `tests/common/log.rs`; the unpark case runs with the namespace fixture from `vpn.py`
+- *Tests:* `daemon.py::{ready_notify, parked_start, sigterm_drains, panic_exits_70,
+  second_serve_refused}` (the first daemon cases of the P1-07 skeleton, which spawns the binary with
+  a fake `NOTIFY_SOCKET`); the unpark case runs with the namespace fixture from `vpn.py`
```

```diff
--- PLAN.md (line 4005)
-| P1-04 | P1-03 | P1-07, P1-11, P1-15, P1-17, P2-07 |
+| P1-04 | P1-03 | P1-11, P1-15, P1-17, P2-07 |
```

```diff
--- PLAN.md (line 4007)
-| P1-06 | P1-03, P1-05 | P1-07, P1-08, P1-09, P3-04, P3-05 |
-| P1-07 | P1-04, P1-06, P0-15 | P1-08, P1-09, P1-16, P2-06, P2-08, P4-10 |
+| P1-06 | P1-03, P1-05 | P1-08, P1-09, P3-04, P3-05 |
+| P1-07 | P0-15 | P1-03, P1-08, P1-09, P1-16, P2-06, P2-08, P4-10 |
```

The level table and the critical paths of §9.4 are regenerated by `scripts/dagcheck.py` after
A10, B10, C2 and C3 are applied (the integrator re-runs it; the numbers in the text change).

### A11. "Receipt" is used nine times and defined nowhere

Locations: P0-01 "record its SHA256 in the P0-01 receipt" (line 2672), P0-03 "saved in the P0-04
receipt" (2887), P3-11 "logged in the receipt" (3627), §9.7 "record them in the M1 receipt" (4191),
§8.3 "validation receipts". §8.3 keeps `docs/validation/<date>/README.md` in git but never says
that a receipt is a section of it, so a fresh agent on P0-01 would create a new file in a new
place. Appendix D has no entry.

```diff
--- PLAN.md (line 4934)
 | T1 / T2 / T3 | Feature tiers: must, should, later (§6.1). |
+| Receipt | The record of an item's or milestone's verification: a section `## <ID>` in `docs/validation/<date>/README.md` (the file §8.3 keeps in git) listing the commands run with their exit codes, the `target/test-logs/` paths, and any manual observation or decision (for example the chord chosen in P2-07). Nothing else is a receipt, and no receipt carries files over 256 KB. |
```

```diff
--- PLAN.md (line 2819)
   reviews and checklists are not items; scripts are.
+- **Receipts:** Appendix D defines the word; an item that names a receipt writes that section in the
+  same change. A receipt records what was run and seen; it is not a report.
```

### A12. The three UI contract files have no specified shape

Locations: `contracts/ui/states.json` (§4.3.4, §5.5.3, P2-08, DL-32), `contracts/ui/themes/expected.json`
(§4.2.1, P2-08, P4-05), `plugin/oliver.torrents/keymap.json` (§4.12, §5.4, P3-15, P4-02, Appendix
B). Each is read by two implementations (QML and Rust) written by different items, possibly by
different agents; the first author invents the schema and the second reverse-engineers it. Three
sentences fix it.

```diff
--- PLAN.md (line 1700)
 earlier").
+
+`contracts/ui/states.json` is a JSON array in this table's row order; each element is
+`{"state": "downloading", "qml_glyph": "U+F041A", "text_glyph": "↓", "word": "Downloading",
+"short": "Down", "secondary": "rate · eta · peers"}`. Glyphs are written as `U+XXXX` codepoints
+(P2-04 pins the MDI ones), so the file is font-independent; the `ready`, `disk_full`,
+`added_before`, `in_list` and `pending` rows use the same shape with those names in `state` and
+the pending verbs as a `verbs` array on the `pending` row.
```

```diff
--- PLAN.md (line 1503)
-  the owner's, with the expected dim hex and ratios in `expected.json`, so the tests do not depend
-  on which themes a CI box has installed). P2-08's `contrast.py` computes, for every theme under
+  the owner's, with `expected.json` holding one object per theme file:
+  `{"owner": {"foreground": "#ffffff", "background": "#000000", "accent": "#8d8d8d", "dim": "#b6b6b6",
+  "dim_rung": 1, "dim_ratio": 10.6, "fg_ratio": 21.0, "accent_ratio": 5.9, "accent_text": false,
+  "low_chroma": true}}`, written by `contrast.py`, so the tests do not depend
+  on which themes a CI box has installed). P2-08's `contrast.py` computes, for every theme under
```

```diff
--- PLAN.md (line 4657)
-hand-maintained here and in `plugin/oliver.torrents/keymap.json`.
+hand-maintained here and in `plugin/oliver.torrents/keymap.json`, a JSON array in this table's row
+order whose elements are `{"id": "torrents.remove", "label": "Remove, keep files", "scope": "list",
+"keys": {"tui": ["d", "x", "Delete"], "dashboard": ["d", "x", "Delete"], "popup": ["d", "x",
+"Delete"]}}`; a surface that does not offer the action has `null` there. P4-02 generates exactly this
+shape from the action table, so P3-15's hand-written file needs no migration.
```

### A13. Plugin test scripts live at three different paths

Locations: `plugin/tests/lint.sh` (P0-11 line 3001, P2-04 line 3429, P2-05 line 3438, P4-01 line
3713, §9.5, §9.6); `plugin/oliver.torrents/tests/…` (§4.1 tree line 1419, §4.16 lines 2120 and
2126, P2-05 line 3433, Appendix E rows 4970-4977 including `…/tests/lint.sh`); bare `tests/qs-smoke.sh`,
`tests/qs-visual.sh`, `tests/VISUAL.md`, `tests/README.md` (§4.16 lines 2124, 2132, 2138, 3443).
One of these is also wrong in substance: `plugin/oliver.torrents/` is symlinked into
`~/.config/omarchy/plugins/` and the shell reloads every plugin on any file change under it
(§4.15), so editing a test under that folder restarts the owner's bar. Pick `plugin/tests/` for
everything, outside the symlinked folder.

```diff
--- PLAN.md (line 1419)
-  tests/             fixture runner, smoke shells and the visual capture list (never loaded by omarchy-shell)
+  (tests live in plugin/tests/, outside this folder: it is symlinked into ~/.config/omarchy/plugins/
+   and any file change under it reloads the shell, §4.15)
```

```diff
--- PLAN.md (line 2120)
-- **Store fixtures** (P2-02): `qs -p plugin/oliver.torrents/tests/store` loads `Store.js` and the
+- **Store fixtures** (P2-02): `qs -p plugin/tests/store` loads `Store.js` and the
```

```diff
--- PLAN.md (line 2124)
-- **Smoke against a disposable daemon** (P2-06): `tests/qs-smoke.sh` creates temporary XDG
+- **Smoke against a disposable daemon** (P2-06): `plugin/tests/qs-smoke.sh` creates temporary XDG
   directories, starts `fbittorrent serve --idle-exit-secs 600 --no-notify` there, and runs
-  `timeout 60 qs -p plugin/oliver.torrents/tests/smoke`. The smoke shell instantiates
+  `timeout 60 qs -p plugin/tests/smoke`. The smoke shell instantiates
```

```diff
--- PLAN.md (line 2132)
-- **Visual captures** (P2-08): `tests/qs-visual.sh` (opt-in, `FBT_TEST_VISUAL=1`) runs against
+- **Visual captures** (P2-08): `plugin/tests/qs-visual.sh` (opt-in, `FBT_TEST_VISUAL=1`) runs against
```

```diff
--- PLAN.md (line 2138)
-  back. The list of states to capture is `tests/VISUAL.md` (every row of §4.11 plus each popup
+  back. The list of states to capture is `plugin/tests/VISUAL.md` (every row of §4.11 plus each popup
```

```diff
--- PLAN.md (line 3433)
-- *Do:* §4.2.1 lint in CI (literal colours, sizes, durations); `plugin/oliver.torrents/tests/VISUAL.md`
+- *Do:* §4.2.1 lint in CI (literal colours, sizes, durations); `plugin/tests/VISUAL.md`
```

```diff
--- PLAN.md (line 4970)
-| `plugin/oliver.torrents/tests/qs-store.sh` | Store fixtures | qs | opt-in `FBT_TEST_QS=1` | P2-02 |
+| `plugin/tests/qs-store.sh` (every `plugin/tests/` script lives outside the symlinked plugin folder, A13) | Store fixtures | qs | opt-in `FBT_TEST_QS=1` | P2-02 |
```

(The `…/tests/` rows that follow it in Appendix E then read as `plugin/tests/…`; no other edit.)

### A14. Four names for the same four limit numbers

Locations: §6.3 P5-04 row (line 2577) names config keys `download_limit`, `upload_limit`,
`alt_download_limit`, `alt_upload_limit`, `alt_enabled`; A.9 `Settings` (line 4588) has
`download_limit_bps?`, `upload_limit_bps?`, `alt_download_limit_bps?`, `alt_upload_limit_bps?` and
no alt switch at all; A.9 `Limits` has `download_bps?` … ; `transfer.set_limits` takes
`download_bps` … . `Settings` mirrors the config file (§3.4.3), so the config keys must be the
`Settings` names, and whether the turtle switch survives a daemon restart is undefined because
nothing persists it. Decision proposed: config and `Settings` share `*_limit_bps` plus
`alt_limits_on: bool`; `Limits` keeps the short `*_bps` names (it is a value object, not a key set).

```diff
--- PLAN.md (line 2577)
-| Global and alternative limits (P5-04) | `session.ratelimits.set_download_bps/set_upload_bps` live; config `download_limit`, `upload_limit`, `alt_download_limit`, `alt_upload_limit`, `alt_enabled`. The global upload cap never lifts the per-torrent 32 KiB/s default | `transfer.set_limits`, `transfer.set_alt {on}`; `summary.alt`, `summary.limits` | `z` toggles; F6 edits; tab bar shows the turtle glyph when on | `z`, popup hero button, dashboard top bar indicator, Settings sheet | `limits [--down N] [--up N] [--alt on|off] [--alt-down N] [--alt-up N]` | Unit: limits applied to `Limits`; `daemon.py::alt_limit_holds`: with a 64 KiB/s alt limit a local transfer stays within ±25 % over 20 s |
+| Global and alternative limits (P5-04) | `session.ratelimits.set_download_bps/set_upload_bps` live; config keys with the `Settings` names of A.9: `download_limit_bps`, `upload_limit_bps`, `alt_download_limit_bps`, `alt_upload_limit_bps`, `alt_limits_on` (so the turtle state survives a restart; `summary.alt` mirrors it). The global upload cap never lifts the per-torrent 32 KiB/s default | `transfer.set_limits`, `transfer.set_alt {on}`; `summary.alt`, `summary.limits` | `z` toggles; F6 edits; tab bar shows the turtle glyph when on | `z`, popup hero button, dashboard top bar indicator, Settings sheet | `limits [--down N] [--up N] [--alt on|off] [--alt-down N] [--alt-up N]` | Unit: limits applied to `Limits`; `daemon.py::alt_limit_holds`: with a 64 KiB/s alt limit a local transfer stays within ±25 % over 20 s; `daemon.py::alt_survives_restart` |
```

```diff
--- PLAN.md (line 4588)
-| `Settings` | `max_active_downloads: u64`, `peer_encryption: enum(off\|prefer\|require)`, `network_interface: str?`, `save_path: str`, `seed_by_default: bool`, `autostart: bool`, `stop_policy: enum(idle\|last-ui\|explicit)`, `idle_exit_secs: u64`, `notify: bool`, `timeout_secs: u64`, `handler: enum(add\|confirm)`, `open_command: str` (default `xdg-open`, §6.2 row 40), `clipboard_watch: bool`, `peer_port: u64`, `show_quality: str`, `download_limit_bps?`, `upload_limit_bps?`, `alt_download_limit_bps?`, `alt_upload_limit_bps?`, `restart_class: [str]` (the keys whose change restarts the engine) |
+| `Settings` | `max_active_downloads: u64`, `peer_encryption: enum(off\|prefer\|require)`, `network_interface: str?`, `save_path: str`, `seed_by_default: bool`, `autostart: bool`, `stop_policy: enum(idle\|last-ui\|explicit)`, `idle_exit_secs: u64`, `notify: bool`, `timeout_secs: u64`, `handler: enum(add\|confirm)`, `open_command: str` (default `xdg-open`, §6.2 row 40), `clipboard_watch: bool`, `peer_port: u64`, `show_quality: str`, `download_limit_bps?`, `upload_limit_bps?`, `alt_download_limit_bps?`, `alt_upload_limit_bps?`, `alt_limits_on: bool` (the persisted turtle switch; `transfer.set_alt` writes it), `restart_class: [str]` (the keys whose change restarts the engine). Every key is also the `config.toml` key: one vocabulary for the file, the API and `fbittorrent settings` |
```

### A15. R15's mitigation names a test that no item runs, and `finishes_in_s` has no formula

Locations: R15 (line 4260) "the fixture test covers a two-file torrent"; Appendix E.3 lists the
two-file fixture for P5-05; P5-05's *Tests* (lines 3893-3900) name no such case. A.9 `Playable`
(line 4580) gives the `stall_free` inequality but not what `finishes_in_s` is, although §3.5 and
§4.11 print it ("finishes in 18 min").

```diff
--- PLAN.md (line 3895)
-  `daemon.py::{playable_before_complete, playable_never_for_iso, playable_event_once,
+  `daemon.py::{playable_before_complete, playable_never_for_iso, playable_event_once,
+  playable_two_file_mid_piece (the E.3 two-file fixture: the prefix is computed from the video
+  file's own first piece, not torrent piece 0; R15),
```

```diff
--- PLAN.md (line 4580)
-| `Playable` | `file: u64`, `ready: bool` (the DL-26 threshold), `stall_free: bool?` (at the current `dl_bps`, the rest of the file arrives before playback reaches the end of the prefix: `prefix_bytes / bitrate ≥ (file_size − prefix_bytes) / dl_bps`; absent when no duration is known or the torrent is not downloading), `prefix_bytes: bytes`, `buffered_s: u64?`, `finishes_in_s: u64?`, `acked: bool` (§4.5 icon acknowledgement) |
+| `Playable` | `file: u64`, `ready: bool` (the DL-26 threshold), `stall_free: bool?` (at the current `dl_bps`, the rest of the file arrives before playback reaches the end of the prefix: `prefix_bytes / bitrate ≥ (file_size − prefix_bytes) / dl_bps`; absent when no duration is known or the torrent is not downloading), `prefix_bytes: bytes`, `buffered_s: u64?` (`prefix_bytes / bitrate`, with `bitrate = file_size / duration`), `finishes_in_s: u64?` (`(file_size − prefix_bytes) / dl_bps`, the right-hand side of the rule; absent when `dl_bps` is absent or 0), `acked: bool` (§4.5 icon acknowledgement) |
```

### A16. Numbers that drifted between the text and the script

Locations: §9.4 (line 4120) "17 items deep, weight 35" while `review-log.md`'s final `dagcheck.py`
output says "16 items, weight 35"; §0.5 (line 191) "the 15 UX defects" while §5.7 and §9.2 say
17. Small, but a fresh agent counting items against the script would stop to reconcile.

```diff
--- PLAN.md (line 4120)
-Critical path to the first demo (17 items deep, weight 35 in S-units with S=1, M=2, L=4;
+Critical path to the first demo (as `scripts/dagcheck.py` prints it after every edit to §9.3; at
+round 2: 16 items, weight 35 in S-units with S=1, M=2, L=4;
```

```diff
--- PLAN.md (line 191)
-   **P4:** turn the TUI into a pure API client, split its 3,504-line module and fix the 15 UX
+   **P4:** turn the TUI into a pure API client, split its 3,504-line module and fix the 17 UX
```

### A17. P1-12's glance test takes five minutes of wall time

Locations: P1-12 *Accept* says every case runs with `idle_exit_secs = 2`; its `glance_pauses`
case (lines 3283-3286) opens a glance "at 240 s remaining, waits 30 s" and asserts an exit "240 s
after the close", which needs the 300 s default and a 4.5-minute wait in CI's 3-minute budget
(P1-07). The semantics are the same at 30 s.

```diff
--- PLAN.md (line 3283)
   bar_does_not, parked_keeps_alive, last_ui, explicit_never, glance_pauses (open a glance at 240 s
-  remaining, wait 30 s, close, assert `idle_exit_in_s` resumes at 240 and the exit happens 240 s
-  after the close, not 210), glance_drop_resumes (kill the glancing client; the countdown resumes)}`;
+  remaining, wait 30 s, close, assert `idle_exit_in_s` resumes at 240 and the exit happens 240 s
+  after the close, not 210; this one case runs with `idle_exit_secs = 30`, so read "20 s remaining,
+  wait 5 s, resumes at 20, exits 20 s after the close, not 15"), glance_drop_resumes (kill the
+  glancing client; the countdown resumes)}`;
```

### A18. Two frame budgets for one mechanism

Locations: P3-09 (line 3595) "no frame over 32 ms in the `QSG_RENDER_TIMING=1` log"; P3-17 (line
3687) and §6.3 (line 2594) "no dropped frame", which is undefined (16.7 ms at 60 Hz? the shell's
own threshold?). One number.

```diff
--- PLAN.md (line 3687)
-  the shell's frame log shows no dropped frame while 60 posters load; Enter on a cell stages the
+  the `QSG_RENDER_TIMING=1` log shows no frame over 32 ms (P3-09's budget) while 60 posters load; Enter on a cell stages the
```

```diff
--- PLAN.md (line 2594)
-| Poster grid (P3-17) | none (browse lists already carry IMDb ids, `src/qbt.rs:100-102`) | existing `search.browse` + `imdb.lookup` | n/a (lists stay rows in the TUI) | `BrowseGrid`: 3 to 6 columns of 2:3 poster cells with title, year, ★ rating and seeds; Enter stages the add; `v` toggles grid/rows | n/a | qs smoke with the poster fixture; a render-time budget assertion (60 posters load without a dropped frame in the shell's own frame log) |
+| Poster grid (P3-17) | none (browse lists already carry IMDb ids, `src/qbt.rs:100-102`) | existing `search.browse` + `imdb.lookup` | n/a (lists stay rows in the TUI) | `BrowseGrid`: 3 to 6 columns of 2:3 poster cells with title, year, ★ rating and seeds; Enter stages the add; `v` toggles grid/rows | n/a | qs smoke with the poster fixture; a render-time budget assertion (no frame over 32 ms in the `QSG_RENDER_TIMING=1` log while 60 posters load, the P3-09 budget) |
```

### A19. §4.2.4 keeps a popup `r` key that Appendix B does not have

Location: §4.2.4 glyph table (line 1579) "refresh: kept for `r` in the popup (resubscribe)";
Appendix B's popup column has no `r`, and §4.10 makes "Refresh" a button on the unknown-result
message. Appendix B is canonical (§4.12); align the glyph table.

```diff
--- PLAN.md (line 1579)
-| refresh | `󰑐` | `Panel.qml:225` | kept for `r` in the popup (resubscribe) |
+| refresh | `󰑐` | `Panel.qml:225` | no longer a key (pushed state needs no refresh; Appendix B has no `r` in the popup); kept only as the glyph of §4.10's "Refresh" button after an unknown mutation result |
```

### A20. `src/api/client.rs` has no owning item

Location: §3.3 lists `src/api/client.rs` ("blocking client for the CLI and TUI; `ensure_daemon()`");
P1-04 creates `ensure_daemon()`; P1-11 "CLI commands use `api::Client`"; P1-15, P1-18 and P4-03
reuse it; no item says it writes the client (connect, `hello`, ids, subscribe iterator, reconnect).
Assign it to P1-11, the first consumer.

```diff
--- PLAN.md (line 3245)
-- *Do:* CLI commands use `api::Client` with the fallbacks of §3.2.3 (Appendix F is the mapping);
+- *Do:* `src/api/client.rs`: the blocking NDJSON client (connect with the 200 ms timeout, `hello`, requests matched to responses by `id` with at most 8 in flight, a `subscribe` iterator that yields frames and tracks `rev`, the reconnect backoff of §3.2.11), reused unchanged by the TUI (P1-15, P4-03) and by `events` (P1-18); CLI commands use it with the fallbacks of §3.2.3 (Appendix F is the mapping);
```

### A21. A right click on the bar icon while the popup is closed has nowhere to show the offer strip

Locations: §4.5 (line 1804) "right adds from the clipboard (as today)"; §4.8 and DL-35 make the
popup's right click show the offer strip before adding. The strip is popup content, so a right
click must open the popup. Say so, and stop saying "as today" (today adds at once).

```diff
--- PLAN.md (line 1804)
-Clicks: left toggles the popup; right adds from the clipboard (as today, `Panel.qml:149`); middle
+Clicks: left toggles the popup; right opens the popup with the quick-add offer strip of §4.8 already showing (today it adds at once, `Panel.qml:149`); middle
```

### A22. One `interval_ms` per `subscribe` call versus per-topic defaults

Locations: A.6 `subscribe` (line 4513) takes one `interval_ms` "default 1000"; A.7 gives per-topic
defaults (`search:<id>` 500 ms, `torrent:<h>.pieces` 2,000 ms, `trackers` 5,000 ms). Which wins
when both exist is a guess. Rule: the call's value applies to every topic of that call; when
omitted, each topic uses its A.7 default.

```diff
--- PLAN.md (line 4513)
-| `subscribe` | `topics: [str]` (1..8), `interval_ms: u64?` (250..10000, default 1000) | `{sub: u64, topics: [str]}` | no | P1-08 |
+| `subscribe` | `topics: [str]` (1..8), `interval_ms: u64?` (250..10000; applies to every topic of the call; when omitted, each topic uses its A.7 default) | `{sub: u64, topics: [str]}` | no | P1-08 |
```

### A23. §8.2 "option B" rests on a wrong size premise

Locations: §8.2 (lines 2703-2706), P0-02 (lines 2869-2870), DL-46 (line 4313), Q3 (line 4331) all
say a `cargo package`-shaped copy of Asupersync would drop "the examples, assets and test fixtures
that make up most of the 57 MB". Measured on 2026-10-18: `vendor/asupersync/src` is 56 MB in 959
`.rs` files; `examples/`, `assets/` and `tests/` are under 100 KB together. `cargo package` always
ships `src/`, so option B is the full tree. The choice is therefore binary (public fork now, or
56 MB in history forever), and it must be made before C3 is committed because P0-04 pushes the
history (C1).

```diff
--- PLAN.md (line 2703)
-Fallback if the owner does not want a separate repository: option B, commit the patched crate
-**as `cargo package` would publish it** (`vendor/asupersync/` without `examples/`, `assets/` and
-the test fixtures that make up most of the 57 MB; measure with `cargo package --list`
-before deciding) as its own commit. A git submodule is rejected: it complicates clones and builds
+Fallback if the owner does not want a separate repository: option B, commit `vendor/asupersync/`
+as it is, as its own commit before C3 (`src/` alone is 56 MB in 959 `.rs` files, measured with
+`du` on 2026-10-18; `examples/`, `assets/` and `tests/` are under 100 KB together, so a
+`cargo package`-shaped copy is not smaller and the choice is the fork or the full tree). A git
+submodule is rejected: it complicates clones and builds
```

```diff
--- PLAN.md (line 2869)
-  the wheel fix upstream. If the owner declines, fall back to §8.2 option B (the
-  `cargo package`-shaped copy of the patched crate, committed with the sources in C3).
+  the wheel fix upstream. If the owner declines, fall back to §8.2 option B (the full
+  `vendor/asupersync/` tree as its own commit before C3; 56 MB, once, in history).
```

```diff
--- PLAN.md (line 4313)
-| DL-46 (round 2) | Commit C3 carries sources, manifest, lock and the `[patch]` entry together (no unbuildable intermediate); option B for Asupersync is a `cargo package`-shaped copy | §8.1, §8.2 |
+| DL-46 (round 2, corrected round 3) | Commit C3 carries sources, manifest, lock and the `[patch]` entry together (no unbuildable intermediate); option B for Asupersync is the full vendored tree (a packaged copy is not smaller: `src/` is 56 MB) | §8.1, §8.2 |
```

```diff
--- PLAN.md (line 4331)
-| Q3 | May a public fork `odcpw/asupersync` hold the 690-line patch, or should a copy be committed: the full 57 MB tree, or a `cargo package`-shaped copy without examples and fixtures (§8.2 option B)? | the fork; packaged copy as the fallback |
+| Q3 (**answer before P0-04 commits C3**: it decides what goes into history) | May a public fork `odcpw/asupersync` hold the 690-line patch (you create the repository; builds need network once), or should the full 56 MB `vendor/asupersync/` tree be committed (§8.2 option B; no second repository, no network, 56 MB in every clone)? | the fork; the full tree as the fallback |
```

## Section B. Cuts

Each cut names what goes, why it costs effort without changing what the owner gets, and what is
preserved. No user-facing behaviour is removed.

### B1. The Rust twin of the daemon harness (`tests/daemon_core.rs`, `tests/common/log.rs`)

What: a second process-level harness in Rust with its own JSONL logging implementation, used by
exactly one item (P1-03's five cases). Why it costs: two spawners, two log writers, two tails-on-
failure, two summaries to keep in the §9.1 shape; after A10 `daemon.py` exists before P1-03 and
can hold those five cases. Preserved: every case, the JSONL contract, `fbtlog_test.py`. (The P1-03
*Tests* edit is in A10.)

```diff
--- PLAN.md (line 959)
-tests/common/log.rs            the Rust side of the test logging contract (§9.1)
```

```diff
--- PLAN.md (line 2806)
-  marker lines (`FBTQS-STEP {json}`) are collected into the same shape; Rust integration tests
-  use `tests/common/log.rs` to write the same lines. A failure prints both file paths and their
+  marker lines (`FBTQS-STEP {json}`) are collected into the same shape by the harness; there is no
+  Rust twin, because every process-level case lives in `daemon.py`. A failure prints both file paths and their
```

```diff
--- PLAN.md (line 3043)
-  timing; `tests/common/log.rs` with the same shape for Rust integration tests; a `qs` shim that
+  timing; a `qs` shim that
```

```diff
--- PLAN.md (line 3049)
 - *Tests:* `fbtlog_test.py`: a fake case that fails writes the expected lines and the summary
-  script counts it; the Rust helper has a unit test writing to a temp dir.
+  script counts it.
```

```diff
--- PLAN.md (line 4963)
-| `tests/daemon_core.rs` | process-level daemon tests (ready, parked, signals, panic, lock) | cargo | yes | P1-03 |
```

```diff
--- PLAN.md (line 4166)
 contract: exit codes, envelopes, determinism, stdout/stderr split, events, api) ·
-`tests/daemon_core.rs` (process-level daemon tests) · `qs-store.sh` (Store fixtures) ·
+`qs-store.sh` (Store fixtures) ·
```

### B2. Legacy-binary compatibility tests (`native.py --legacy-cli`, the v0.1 widget acceptance, `old_engine_sock_reads_only`)

What: P1-10 is accepted by running "the pinned v0.1.2 CLI and widget" against the v0.2 daemon, and
P1-11 tests the new CLI against a v0.1 TUI-hosted engine. Why it costs: one binary serves daemon,
CLI and widget, and `scripts/install.sh` replaces it in one `mv`; on this machine no v0.1 CLI or
widget will ever meet a v0.2 daemon, and the only moment a v0.2 CLI meets a v0.1 TUI-hosted
`engine.sock` is the first upgrade, once, by the owner. Keeping a pinned old binary buildable and
driving it through `native.py` is a second product to maintain for a one-time event. Preserved:
the compat adapter itself (the TUI of P1-15 is its real consumer until P4-03, and `native.py`
exercises it there), the write-refusal guard in §3.6 (one `if`), checked by hand during the first
upgrade and written in the M1a receipt (C2).

```diff
--- PLAN.md (line 3230)
-- *Why:* the v0.1 CLI and the v0.1 bar widget keep working during the migration (§3.6). The v0.1
-  TUI cannot attach to any engine but its own (`src/main.rs:787,804-806`); it becomes a client in
-  P1-15.
-- *Accept:* every CLI call `native.py` makes today (`status`, `list`, `add`, `stop`, `start`,
-  `delete`, `rss …`, `capabilities`) answers identically from a v0.1 binary pointed at the
-  daemon's `engine.sock`; `plugin/oliver.torrents/Service.qml` (v0.1, `status --json` polling)
-  renders the daemon's torrents. A v0.1 `fbittorrent tui` is out of scope: it sees a live
-  `engine.sock` and runs `backend::open_app()` (`src/main.rs:787`), and nothing the daemon can
-  answer changes that; `install.sh` replaces the binary, so no v0.1 TUI remains after the upgrade.
-- *Tests:* `native.py --legacy-cli <v0.1 binary> --daemon <v0.2 binary>` (runs the CLI steps of the
-  11 cases against the daemon with the pinned v0.1.2 binary from the `M0` tag); unit
-  `legacy::tests::route_parse_roundtrip`; the TUI-as-client cases move to P1-15.
+- *Why:* the TUI keeps `qbt::Client` and the 4-byte framing until P4-03 (§5.2 step 1), so the
+  daemon must answer that framing; it is the adapter's only consumer. One binary serves daemon, CLI
+  and widget, so no v0.1 CLI or widget survives `install.sh`, and the v0.1 TUI cannot attach to any
+  engine but its own (`src/main.rs:787,804-806`).
+- *Accept:* every `qbt::Client` call the TUI makes (`torrents/info`, `files`, `add`, `start`,
+  `stop`, `delete`, `filePrio`, `setUploadLimit`, `setShareLimits`, `app/*`) answers from the
+  daemon's `engine.sock` with the qBittorrent-shaped JSON of `src/engine/view.rs`; `app/shutdown`
+  stops the daemon and `app/restart` restarts the engine.
+- *Tests:* unit `legacy::tests::route_parse_roundtrip`; `native.py`'s 11 cases run against the
+  TUI-as-client in P1-15 and cover every route above.
```

```diff
--- PLAN.md (line 1286)
-are harmless, writes against an unknown engine are not. The compat adapter is tested by running
-the pinned v0.1.2 CLI and widget against the v0.2 daemon (P1-10); the v0.1 TUI owns its engine and
-never attaches (`src/main.rs:787`), so after installing v0.2 the only TUI is the new one (P1-15).
+are harmless, writes against an unknown engine are not. This can happen once, during the first
+upgrade, and is checked by hand then (M1a receipt); the compat adapter's consumer is the TUI of
+P1-15 and `native.py` tests it there. The v0.1 TUI owns its engine and never attaches
+(`src/main.rs:787`), so after installing v0.2 the only TUI is the new one.
```

```diff
--- PLAN.md (line 3260)
 - *Tests:* `agent_contract.py::{R-001…R-0xx (ported), exit_code_dictionary_total, reads_never_start,
   json_determinism, stdout_stderr_split, status_stopped_shape, version_skew_warning,
-  old_engine_sock_reads_only, wait_survives_upgrade_restart, wait_exits_8_when_daemon_idles_out}`
-  (`old_engine_sock_reads_only` runs the new CLI against a v0.1 TUI-hosted `engine.sock` and
-  asserts reads succeed and writes are refused, §3.6; the two `wait` cases cover an exit-75 restart
+  wait_survives_upgrade_restart, wait_exits_8_when_daemon_idles_out}`
+  (the two `wait` cases cover an exit-75 restart
```

```diff
--- PLAN.md (line 4964)
-| `regression_tests/native.py` | isolated TUI/CLI today; `--legacy-cli <v0.1> --daemon <v0.2>` mode later (P1-10) | tmux, util-linux | yes | exists; P0-15, P1-10 |
+| `regression_tests/native.py` | isolated TUI/CLI; from P1-15 the TUI runs as a client of a disposable daemon | tmux, util-linux | yes | exists; P0-15, P1-15 |
```

```diff
--- PLAN.md (line 4151)
-| P1 | `all.sh` + `daemon.py` (every P1 case) + `agent_contract.py` + `tests/daemon_core.rs` + `native.py --legacy-cli` (the pinned v0.1.2 CLI against the daemon); `resources.py --daemon` baseline written | The owner clicks a magnet link with no window open and gets a notification, not a terminal |
+| P1 | `all.sh` + `daemon.py` (every P1 case) + `agent_contract.py` + `native.py` (the TUI as a client); `resources.py --daemon` baseline written | The owner clicks a magnet link with no window open and gets a notification, not a terminal |
```

### B3. The 1-pixel automated comparison against the Wi-Fi panel's screenshot

What: `qs-visual.sh --compare` reads "the hero title baseline, meta baseline, first
`PanelSeparator` and row pitch" of a first-party panel by scanning row boundaries in its PNG and
fails above 1 px. Why it costs: it is image analysis of someone else's rendering (anti-aliased
text, theme alpha, a panel whose layout Omarchy may change in any release), a flaky gate whose
failure says nothing about what to change, and a self-test ("comparing the Wi-Fi panel with itself
reports 0 px") that proves only the scanner's determinism. The owner's complaint is look and feel;
the useful artefact is the two captures side by side in the same theme, judged by the owner, which
the plan already requires ("captures are reviewed by a person"). Preserved: the capture, the
side-by-side image, our own geometry through `state()`, the pixel-diff against our previous
capture, the §0.2 target.

```diff
--- PLAN.md (line 79)
-| Look | A careful TUI prototype [RC §9]; a popup built from shell components but fed by polling | Popup and dashboard built only from shell tokens and components (lint), with the popup's hero title baseline, meta baseline, first separator and row pitch within 1 px of the Wi-Fi panel's in the same theme (`qs-visual.sh --compare omarchy.network`), every §4.11 state captured in the owner's theme and one light theme; a TUI whose state column reads at 80 columns on a monochrome theme (goldens) | §4, §5, P2-08 |
+| Look | A careful TUI prototype [RC §9]; a popup built from shell components but fed by polling | Popup and dashboard built only from shell tokens and components (lint), with the popup captured side by side with the Wi-Fi panel in the same theme and judged by the owner against it (`qs-visual.sh --compare omarchy.network`; our hero, meta, separator and row geometry printed under the pair), every §4.11 state captured in the owner's theme and one light theme; a TUI whose state column reads at 80 columns on a monochrome theme (goldens) | §4, §5, P2-08 |
```

```diff
--- PLAN.md (line 2141)
-  stands out. `--compare <plugin>` (for example `omarchy.network`) captures the named first-party
-  panel in the same theme right after ours and reads, through each panel's `state()` hook (ours)
-  and the capture's row-boundary scan (theirs), the y of the hero title baseline, the meta
-  baseline, the first `PanelSeparator` and the pitch of the first two rows; it prints both columns
-  side by side and fails when any differs by more than 1 px. This is what "looks first-party"
-  means as a test.
+  stands out. `--compare <plugin>` (for example `omarchy.network`) captures the named first-party
+  panel in the same theme right after ours and writes the two PNGs side by side into one image,
+  with our hero title baseline, meta baseline, first `PanelSeparator` y and row pitch (read through
+  `state()`) printed under it. A person judges the pair; there is no pixel gate on someone else's
+  panel, because anti-aliasing and theme alpha make a 1 px scan flaky and a failing scan would not
+  say what to change. "Looks first-party" is the owner's verdict on that image, recorded in the
+  receipt.
```

```diff
--- PLAN.md (line 3472)
-  daemon and restores the hook; `qs-visual.sh popup downloading --compare omarchy.network`
-  captures the Wi-Fi panel in the same theme and reports its hero title baseline, meta baseline,
-  first `PanelSeparator` y and first-two-row pitch next to ours, failing on a difference above 1
-  px; `contrast.py` passes for every installed theme under the computed dim (or lists the failing
+  daemon and restores the hook; `qs-visual.sh popup downloading --compare omarchy.network`
+  writes the pair image with our geometry printed under it; `contrast.py` passes for every installed theme under the computed dim (or lists the failing
```

```diff
--- PLAN.md (line 3482)
-  --self-test` with a known-bad theme; `qs-visual.sh --compare` against a stored
-  first-party capture (self-test: comparing the Wi-Fi panel with itself reports 0 px); a
+  --self-test` with a known-bad theme; `qs-visual.sh --compare` (self-test: the pair image exists
+  and names both panels); a
```

### B4. `scripts/free-chord.sh` and its unit test

What: a script with a fixture-based unit test to check that one key chord is unused in Hyprland
bindings. Why it costs: it answers a one-time question (which chord to suggest in P2-07) that a
single `grep`, run once and recorded, answers; the unit test tests the grep. Preserved: the check,
the recorded chord.

```diff
--- PLAN.md (line 3452)
-- *Do:* §8.6: `scripts/install.sh`, `setup` flags, the menu entries (D18), a keybinding suggestion
-  with a chord verified free in Omarchy's bindings (SUPER+CTRL+T is taken [FO §2.8]; the check is a
-  script that greps `/usr/share/omarchy/default/hypr/bindings/*.lua` and `~/.config/hypr/`), the
-  README install section.
+- *Do:* §8.6: `scripts/install.sh`, `setup` flags, the menu entries (D18), a keybinding suggestion
+  with a chord checked free once by grepping `/usr/share/omarchy/default/hypr/bindings/*.lua` and
+  `~/.config/hypr/` (SUPER+CTRL+T is taken [FO §2.8]; the chosen chord and the grep output go in
+  the P2-07 receipt), the README install section.
```

```diff
--- PLAN.md (line 3459)
 - *Tests:* `daemon.py::{setup_dry_run_lists_actions, setup_idempotent}` with a temp `HOME` and a
-  `systemctl` shim; `scripts/free-chord.sh` has a unit test with a fixture bindings file.
+  `systemctl` shim.
```

```diff
--- PLAN.md (line 4984)
-| `scripts/free-chord.sh` | the dashboard chord is free in Hyprland bindings | — | yes (fixture) | P2-07 |
```

### B5. CI jobs that lint the plan document, and a doc-link job for files that never change

What: `dagcheck.py` and `mockwidth.py` "run in CI on every change to `docs/plan-2026-10-18/`";
`check-doc-links.sh` joins the `lint` job. Why it costs: the plan stops changing once it is
converted to beads (§10.5), and `docs/validation/` changes only when receipts move (P0-01, once);
a CI job for either is a gate that never gates. Preserved: both plan scripts as review-round tools,
run by hand before each round and before the beads conversion (as this round did); the link check,
run once in P0-01 and by hand when receipts move. (The `scripts/check-fixtures.sh` entry in the
same sentence goes with B6.)

```diff
--- PLAN.md (line 4172)
-fixtures) · `resources.py` (budget gate) · `scripts/check-doc-links.sh`, `scripts/check-fixtures.sh`,
-`scripts/lint-sizes.sh`, `scripts/free-chord.sh` (lints) · `scripts/dagcheck.py` (this plan's §9.3
+fixtures) · `resources.py` (budget gate) · `scripts/check-doc-links.sh` (run by hand when receipts move),
+`scripts/lint-sizes.sh` (lint) · `scripts/dagcheck.py` (this plan's §9.3
 against §9.4: undefined IDs, cycles, orphans, consumed-by consistency, the level table, weighted
 critical paths) · `scripts/mockwidth.py` (every mockup line in §5 and Appendix C measured in cells
-with East Asian width rules and Nerd Font PUA = 1; overflow and ragged boxes fail). Both ran clean
-on this round and run in CI on every change to `docs/plan-2026-10-18/`.
+with East Asian width rules and Nerd Font PUA = 1; overflow and ragged boxes fail). Both ran clean
+on this round; they are review-round tools, run by hand before each round and before the beads
+conversion, not CI jobs: the plan stops changing once it is beads.
```

```diff
--- PLAN.md (line 4979)
-| `scripts/check-doc-links.sh` | markdown links into `docs/validation` | — | yes | P0-01 |
-| `scripts/dagcheck.py` | PLAN.md §9.3 vs §9.4: undefined IDs, cycles, orphans, consumed-by, level table, weighted critical paths | python3 | yes (on `docs/plan-2026-10-18/` changes) | P0-06 (the round-2 reviewer's script, `reviews/round-2-architecture.md` §D) |
-| `scripts/mockwidth.py` | PLAN.md §5 and Appendix C mockups: cell widths, overflow against the declared 80/120/160, ragged boxes | python3 | yes (same trigger) | P0-06 (round 2) |
+| `scripts/check-doc-links.sh` | markdown links into `docs/validation` | — | no (once in P0-01; by hand when receipts move) | P0-01 |
+| `scripts/dagcheck.py` | PLAN.md §9.3 vs §9.4: undefined IDs, cycles, orphans, consumed-by, level table, weighted critical paths | python3 | no (review rounds, by hand) | P0-06 (the round-2 reviewer's script, `reviews/round-2-architecture.md` §D) |
+| `scripts/mockwidth.py` | PLAN.md §5 and Appendix C mockups: cell widths, overflow against the declared 80/120/160, ragged boxes | python3 | no (review rounds, by hand) | P0-06 (round 2) |
```

```diff
--- PLAN.md (line 3858)
 - *Tests:* `scripts/check-doc-links.sh` logs each link and its result to
-  `target/test-logs/docs/links.jsonl` and exits non-zero on a broken link; it joins the `lint` CI
-  job in P0-06.
+  `target/test-logs/docs/links.jsonl` and exits non-zero on a broken link; it runs once here and by
+  hand whenever receipts move (not a CI job: nothing under `docs/validation/` changes between
+  releases).
```

### B6. `scripts/check-fixtures.sh`, a schema lint the round-trip test already is

What: a shell script that validates the fixture files' `dir` vocabulary, in the CI `lint` job.
Why it costs: `api::fixtures::roundtrip` deserializes every fixture line into the typed enums; an
unknown `dir` or a malformed frame already fails there with a line number. Preserved: the check.

```diff
--- PLAN.md (line 3151)
-- *Tests:* `api::fixtures::{roundtrip, no_sentinels, changelog_covers_minors}`; `scripts/check-fixtures.sh`
-  validates the `dir` vocabulary and joins the `lint` job.
+- *Tests:* `api::fixtures::{roundtrip, no_sentinels}` (`roundtrip` deserializes every fixture line
+  into the typed enums, so an unknown `dir` or a malformed frame fails there with its line number;
+  no separate schema script).
```

```diff
--- PLAN.md (line 4982)
-| `scripts/check-fixtures.sh` | fixture `dir` vocabulary and changelog | — | yes | P1-05 |
```

```diff
--- PLAN.md (line 2734)
-| `lint` | `git diff --check`; the QML token lint (P2-05); the plugin structure lint (no `Timer`/`Process` outside `Service.qml`, P2-04); the fixture schema and changelog check (P1-05); the `--json` determinism check (`verify-determinism`, P1-11) | P0-06, P1-05, P2-04, P2-05 |
+| `lint` | `git diff --check`; the QML token lint (P2-05); the plugin structure lint (no `Timer`/`Process` outside `Service.qml`, P2-04); `scripts/lint-sizes.sh` (P4-01); the `--json` determinism check (`verify-determinism`, P1-11) | P0-06, P2-04, P2-05, P4-01 |
```

### B7. `proto_minor` and the per-minor fixture-changelog test

What: a minor protocol number in `hello`, a `docs/IPC.md` changelog entry plus a fixture file per
minor bump, and `changelog_covers_minors` ("a fixture with `proto_minor: 9` fails"). Why it costs:
no client branches on the number; `caps` is the negotiation (§3.4.3) and every op and field already
carries its item in the `Since` column; one repository owns every client. A version counter nobody
reads is a thing to bump, document and test on every addition. Preserved: `proto` (the major, which
clients do check), `caps`, the changelog as prose.

```diff
--- PLAN.md (line 1061)
-- A minor version may add operations, topics, caps, optional result and event fields, and new
+- Additive changes within a major are: operations, topics, caps, optional result and event fields, and new
```

```diff
--- PLAN.md (line 1072)
-- `docs/IPC.md` carries a changelog; every minor bump adds an entry and a fixture file; a test
-  fails when a fixture references a `proto_minor` the changelog does not list.
+- `docs/IPC.md` carries a changelog: one entry per cap or field added, naming the item that added
+  it. There is no minor version number; `caps` is the only feature negotiation (one repository owns
+  every client, and a number nobody branches on is ceremony).
```

```diff
--- PLAN.md (line 902)
-- Minor protocol skew (plugin newer than daemon, or older) is handled by `caps` and `proto_minor`
-  (§3.4.4): a client hides what the daemon cannot serve, and the daemon rejects unknown ops with
+- Feature skew (plugin newer than daemon, or older) is handled by `caps` (§3.4.3): a client hides
+  what the daemon cannot serve, and the daemon rejects unknown ops with
```

```diff
--- PLAN.md (line 1014)
-← {"id":1,"ok":true,"result":{"proto":1,"proto_minor":0,
+← {"id":1,"ok":true,"result":{"proto":1,
```

```diff
--- PLAN.md (line 4405)
-| result | `proto: u64`, `proto_minor: u64`, `server: {version: str, build: str, pid: u64, launcher: enum(systemd\|detached\|foreground), log: str}` (`log` is `journalctl --user -u fbittorrent` or the file path, ready to print as a hint), `engine: str`, `instance: str` (uuid), `engine_epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `caps: [str]` (§3.4.3), `limits: {line_in: u64, line_out: u64, inflight: u64, subs: u64, hashes: u64}`, `policy: {stop: enum(idle\|last-ui\|explicit), idle_exit_secs: u64}` |
+| result | `proto: u64`, `server: {version: str, build: str, pid: u64, launcher: enum(systemd\|detached\|foreground), log: str}` (`log` is `journalctl --user -u fbittorrent` or the file path, ready to print as a hint), `engine: str`, `instance: str` (uuid), `engine_epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `caps: [str]` (§3.4.3), `limits: {line_in: u64, line_out: u64, inflight: u64, subs: u64, hashes: u64}`, `policy: {stop: enum(idle\|last-ui\|explicit), idle_exit_secs: u64}` |
```

```diff
--- PLAN.md (line 4620)
-← {"id":1,"ok":true,"result":{"proto":1,"proto_minor":0,"state":"running","caps":["torrents","files","subscribe","mutations","log"], …}}
+← {"id":1,"ok":true,"result":{"proto":1,"state":"running","caps":["torrents","files","subscribe","mutations","log"], …}}
```

```diff
--- PLAN.md (line 3147)
-  contract test 1 (round-trip), the sentinel scan (§3.4.9) and the changelog check (§3.4.4).
+  contract test 1 (round-trip) and the sentinel scan (§3.4.9).
```

```diff
--- PLAN.md (line 3149)
 - *Accept:* every P1-scope operation and event has a type and at least one fixture; the tests run in
-  `cargo test`; a fixture with `proto_minor: 9` fails the changelog check.
+  `cargo test`.
```

### B8. A unit test that checks code against a planning appendix

What: `api::ops::tests::table_matches_appendix_a` and "`robot-docs api` prints Appendix A". Why it
costs: Appendix A is a section of a planning document that is converted to beads and then frozen;
testing code against it means either editing the plan on every API addition or watching the test
rot. `docs/IPC.md` (P1-05) is the living contract and already has `api_list_matches_ipc_md`.
Preserved: the machine check, against the one living document.

```diff
--- PLAN.md (line 3361)
-  daemon when it is down (`hello` is not reachable then); `robot-docs api` prints Appendix A. A unit
-  test asserts the table matches `docs/IPC.md`'s op list, and `api nope` suggests the nearest names
+  daemon when it is down (`hello` is not reachable then); `robot-docs api` prints the table (the same
+  content `docs/IPC.md` documents). A unit
+  test asserts the table matches `docs/IPC.md`'s op list, and `api nope` suggests the nearest names
```

```diff
--- PLAN.md (line 3375)
-  api_mutating_starts_daemon_read_does_not}`; unit `api::ops::tests::table_matches_appendix_a`.
+  api_mutating_starts_daemon_read_does_not}`; unit `api::ops::tests::table_matches_ipc_md`.
```

```diff
--- PLAN.md (line 4385)
 values to "other"). Every `args` object rejects unknown keys (`invalid_args`, `details.key`).
+
+After P1-05, `docs/IPC.md` is the living contract and the only document tests compare against;
+this appendix is its initial content and is not maintained afterwards.
```

### B9. Self-test modes of the harnesses

What: `daemon.py --self-test` and `tui_smoke.py --self-test`. Why it costs: the plan's own words,
"the harness is covered by every later case"; a stub-driven self-test exercises the harness
against a stub, which no real case does, and `fbtlog_test.py` (P0-15) already covers the one
thing worth testing in isolation, the log shape and the failure tail. Preserved: `fbtlog_test.py`.

```diff
--- PLAN.md (line 3189)
-- *Tests:* `daemon.py --self-test` runs a trivial case through every helper (shims, fixture server,
-  peer pair) and checks the JSONL output; the harness is covered by every later case.
+- *Tests:* the harness is covered by every case that uses it; `fbtlog_test.py` (P0-15) covers the
+  logging shape and the failure tail. No `--self-test` mode.
```

```diff
--- PLAN.md (line 3810)
-- *Tests:* itself (`tui_smoke.py --self-test` runs one step against a stub).
+- *Tests:* its own steps against the disposable daemon; a failing step's JSONL shape is covered by `fbtlog_test.py`'s forced-failure case.
```

### B10. P7-08 and the §7.4 marketplace build design

What: a conditional work item ("only if the owner opts in") with an `omarchy plugin validate`
acceptance, and a §7.4 that designs a `curated-providers` cargo feature, a second plugin
repository and a Torznab-only Shows mode. Why it costs: DL-17 says the marketplace is not a goal;
§2.1 says the product is not aimed at it; the item sits in the DAG as the only orphan and the
feature-flag design is speculative work for a listing the owner has excluded (`just-say-no`: a
gate that gates nothing). Preserved: §7.2 (good engineering regardless), P7-01 Torznab (justified
on its own: private trackers without Python), a two-line note of what a listing would take.

```diff
--- PLAN.md (line 3978)
-| P7-08 | Marketplace readiness, only if the owner opts in | §7.4 | The §7.4 checklist is complete and the plugin repository validates with `omarchy plugin validate` | M | P7-01, P3-16 (conditional; not on any milestone) |
+| P7-08 | Marketplace readiness | **Superseded (round 3): removed as ceremony.** The marketplace is not a goal (DL-17, §2.1); §7.4 keeps a two-line note of what a listing would take, and nothing is designed until the owner asks. | - | - |
```

```diff
--- PLAN.md (line 2655)
-### 7.4 If the owner ever opts in (P7-08)
+### 7.4 If the owner ever asks for a listing
 
-A separate public plugin repository with a root `manifest.json`, README, licence and real
-screenshots; a cargo feature `curated-providers` (default on) that a marketplace build turns off,
-which also hides the browse lists and makes Shows use user-configured Torznab sources; zero
-bundled indexers in that build; the plugin detects a missing or old binary and shows install
-steps; a short security note listing the bounds, file modes, confirms and teardown above.
+Not planned (DL-17; P7-08 superseded in round 3). For the record, a listing would need: a separate
+public plugin repository with a root `manifest.json`, README, licence and real screenshots; a build
+without the curated providers and browse lists; the plugin detecting a missing or old binary; a
+security note listing §7.2's bounds. None of it is designed further until asked.
```

```diff
--- PLAN.md (line 536)
-| Omarchy marketplace listing | §7. | The owner opts in (P7-08). |
+| Omarchy marketplace listing | §7. | The owner asks; §7.4 says what it would take. |
```

```diff
--- PLAN.md (line 4044)
-| P3-16 | P3-10..P3-15, P3-17, P3-18, P2-06 | M2, P7-08 |
+| P3-16 | P3-10..P3-15, P3-17, P3-18, P2-06 | M2 |
```

```diff
--- PLAN.md (line 4081)
-| P7-01 | P3-01 | M6, P7-08 |
+| P7-01 | P3-01 | M6 |
```

```diff
--- PLAN.md (line 4087)
-| P7-08 | P7-01, P3-16 | — (conditional on the owner) |
```

### B11. Duplicate fields: `Summary.free_space`, `Summary.daemon.clients`, and counters persisted to `stats.json`

What: `Summary` carries `free_space` "for v0.1-shaped consumers" next to `disk.free_bytes`, and a
`clients` map that `app.info` already returns; per-client-kind connection counters are written to
`stats.json`. Why it costs: `Summary` is a v1 object with no v0.1 consumer (`status --json` has
no `free_space` today, `src/main.rs:420-425`); two names for one number is the drift the plan
fights elsewhere; persisting the counters serves nobody (P2-01's test reads them live through
`app.info`, and the "retirement review" that would have read history was removed in round 1).
Preserved: `disk`, `app.info.clients` in memory, `stats.json` for P6-04's all-time totals.

```diff
--- PLAN.md (line 4585)
-| `Summary` | `dl_bps?`, `ul_bps?`, `counts: {total, downloading, stalled, metadata, checking, queued, paused, done, seeding, waiting_vpn, error, staged, ready}` (all `u64`), `alt: bool`, `limits: Limits`, `network: {state: enum(ready\|waiting\|off), message: str, interface: str?}`, `daemon: {state: enum(starting\|running\|parked\|stopping), idle_exit_in_s: u64?, busy_reasons: [open(downloading\|seeding\|parked_waiting\|interactive\|search\|mutation)], upgrade_pending: bool, clients: {…}}`, `top: [{hash, name, state, progress?, dl_bps?, eta_s?, playable?}]` (≤ 5), `free_space: bytes?`, `disk: {free_bytes: bytes?, needed_bytes: bytes, low: bool}` (P5-07; `free_space` stays for v0.1-shaped consumers), `history: [u64]?` (last 60 download samples, P6-03) |
+| `Summary` | `dl_bps?`, `ul_bps?`, `counts: {total, downloading, stalled, metadata, checking, queued, paused, done, seeding, waiting_vpn, error, staged, ready}` (all `u64`), `alt: bool`, `limits: Limits`, `network: {state: enum(ready\|waiting\|off), message: str, interface: str?}`, `daemon: {state: enum(starting\|running\|parked\|stopping), idle_exit_in_s: u64?, busy_reasons: [open(downloading\|seeding\|parked_waiting\|interactive\|search\|mutation)], upgrade_pending: bool}`, `top: [{hash, name, state, progress?, dl_bps?, eta_s?, playable?}]` (≤ 5), `disk: {free_bytes: bytes?, needed_bytes: bytes, low: bool}` (P5-07; the one place free space lives), `history: [u64]?` (last 60 download samples, P6-03) |
```

```diff
--- PLAN.md (line 1024)
-the dashboard (§4.4). The daemon counts connections per kind in `stats.json`; nothing leaves the
+the dashboard (§4.4). The daemon counts connections per kind in memory (`app.info.clients`, reset at start); nothing leaves the
```

```diff
--- PLAN.md (line 3304)
-  [--json] [--level] [--source]`; per-client-kind connection counters in `stats.json`; retire
+  [--json] [--level] [--source]`; per-client-kind connection counters in memory (`app.info.clients`); retire
```

### B12. Visual captures stored under `docs/validation/`

What: P2-05 stores every capture under `docs/validation/<date>/plugin-v0.2/` "each under 256 KB",
and §8.3 keeps every file under 256 KB that a tracked markdown links to. Why it costs: §4.11 has
about twenty states, times two themes, times four dashboard widths (P3-16); at 100 KB each that is
10-16 MB of PNGs per release path back into the tree the plan just spent P0-01 emptying. Preserved:
every capture (in `target/test-logs/visual/` and the archive), the README's hero screenshots.

```diff
--- PLAN.md (line 3436)
-- *Accept:* the lint passes in CI and fails on a planted `#ff0000`; captures stored under
-  `docs/validation/<date>/plugin-v0.2/` (each under 256 KB).
+- *Accept:* the lint passes in CI and fails on a planted `#ff0000`; captures stored under
+  `target/test-logs/visual/<date>/` and archived with the receipts (§8.3), never under `docs/`;
+  the README's two hero screenshots are the only committed images.
```

```diff
--- PLAN.md (line 2135)
   the popup or the dashboard, captures the output with `grim` (installed), stores the PNG under
-  `docs/validation/<date>/plugin-v<ver>/<state>-<theme>.png`, and restores the hook. With
+  `target/test-logs/visual/<date>/<state>-<theme>.png` (archived with the receipts of §8.3, never
+  committed), and restores the hook. With
```

### B13. Rules stated twice where one pointer does

Single-source-of-truth inventory (check 1 of the brief), with the proposed canonical home:

| Rule | Occurrences | Canonical | Others become |
|---|---|---|---|
| API messages | §3.4.6 overview, A.4–A.6, Appendix F | A.4–A.6 (then `docs/IPC.md`) | §3.4.6 → one line (hunk below); Appendix F stays (it is the CLI mapping, not a copy) |
| Exit-code mapping | A.3, Appendix F reachability, `src/agent.rs` | A.3 | Appendix F keeps only the reachability list (already so) |
| Keymap | §4.5 labels, §4.12, §5.4, §5.7 row 12, Appendix B, README (generated) | Appendix B | §5.7 row 12 → pointer (hunk below); §4.5 and mockups are labels, fine |
| State vocabulary | §3.5 (derivation), §4.3.4 (display), §5.5.3 (cell rule), A.9 (enum) | §3.5 enum + §4.3.4 table | consistent after round 2; no edit |
| Lifetime / idle | A1, §0.4, §0.6, §3.2.10, Appendix D, DL-02, Q1 | §3.2.10 | A1 prose → pointer (hunk below) |
| Stop policy table | §3.2.10 only | — | — |
| Theme / contrast | §4.2.1, §5.5.1, DL-23, DL-31, P2-08, P4-05 | §4.2.1 (dim), §5.5.1 (accent, low-chroma) | A2 |
| Line caps | §3.4.1, A.1, A.2 `limits` | §3.4.1 | A.1 and `hello.limits` restate the numbers; acceptable (a client needs them at runtime) |
| Revision semantics | §3.4.7, §4.4 rule 11, Appendix D, A.1 | §3.4.7 | consistent; no edit |
| Reconnect backoff, heartbeat | §3.2.11, §3.4.7, §4.4 rules 4 and 7 | §3.2.11 (backoff), §3.4.7 (heartbeat) | consistent; no edit |
| Milestone contents | §9.2, §9.3 milestone lines, §9.4, §9.7, §3.6 versions | §9.3 lines + §9.4 | consistent (C2 adds one) |

```diff
--- PLAN.md (line 1083)
 
-| Namespace | Operations (Appendix A has fields) |
-|---|---|
-| `app` | `hello`, `ping`, `app.info`, `app.client`, `app.shutdown`, `app.restart`, `app.settings.get`, `app.settings.set`, `app.network`, `app.log`, `app.stats`, `app.free_space`, `app.clipboard.dismiss` |
-| `transfer` | `transfer.info`, `transfer.set_limits`, `transfer.set_alt` |
-| `torrents` | `torrents.list`, `torrents.get`, `torrents.add`, `torrents.commit`, `torrents.discard`, `torrents.start`, `torrents.stop`, `torrents.remove`, `torrents.files`, `torrents.set_files`, `torrents.set_seed`, `torrents.set_upload_limit`, `torrents.recheck`, `torrents.peers`, `torrents.trackers`, `torrents.reannounce`, `torrents.pieces`, `torrents.force_start`, `torrents.queue_move`, `torrents.open_target`; later `torrents.move`, `torrents.rename`, `torrents.add_tracker`, `torrents.remove_tracker`, `torrents.set_download_limit` |
-| `search` | `search.providers`, `search.set_provider`, `search.install`, `search.start`, `search.stop`, `search.browse_sources`, `search.browse`, `search.grab` |
-| `rss` | `rss.feeds`, `rss.add`, `rss.remove`, `rss.refresh`, `rss.items`, `rss.set_read`, `rss.mark_feed_read`, `rss.grab` |
-| `shows` | `shows.list`, `shows.follow`, `shows.unfollow`, `shows.set_quality`, `shows.check`, `shows.releases`, `shows.set_read`, `shows.mark_show_read`, `shows.undo_read`, `shows.grab` |
-| `imdb` | `imdb.lookup` |
-| subscriptions | `subscribe`, `unsubscribe` |
+Namespaces: `app`, `transfer`, `torrents`, `search`, `rss`, `shows`, `imdb`, plus `subscribe` and
+`unsubscribe`. The operations are listed once, in A.4 to A.6 (and, after P1-05, in `docs/IPC.md`);
+this section does not repeat them, so the two cannot drift.
```

```diff
--- PLAN.md (line 2439)
-| 12 | Key meanings change by screen | `d`/`x`/Delete remove and `D` deletes files on every list; `f` follows everywhere (`F` stays an alias); `i` always opens IMDb (inline lookup happens on hover); `s` sorts every list; `U` is undo-remove and `u` is seed; `Q` is quit everywhere and `=` is show quality; the remaining per-screen meanings are listed in Appendix B and in Help | P4-02, P4-07 |
+| 12 | Key meanings change by screen | One keymap, Appendix B (DL-34), enforced by the action table of §5.4 (`no_duplicate_key_per_scope`, `global_keys_never_rebound`) | P4-02, P4-07 |
```

```diff
--- PLAN.md (line 93)
-period** (`stop_policy = "idle"`, `idle_exit_secs = 300`). "Work" means an incomplete download
-the user wants running, a torrent the user explicitly allowed to seed, a running search, a
-pending mutation, an open TUI or dashboard, or a VPN-parked engine that still has downloads
-waiting. The bar widget does not count as an open UI. Rationale: the value of a daemon is that
+period** (`stop_policy = "idle"`, `idle_exit_secs = 300`). "Work" is defined by the busy table of
+§3.2.10 and nowhere else; the bar widget does not count as an open UI, and an open popup only
+pauses the countdown. Rationale: the value of a daemon is that
```

## Section C. Improvements

**Fresh-agent walk (check 2 of the brief), in brief.** P0-01 to P0-04: implementable from the text
once "receipt" is defined (A11) and Q3 is answered before C3 (A23, C1). P0-05 to P0-16: no
guesses found; P0-08's two candidate fixes are honestly labelled. P1-01, P1-02, P1-19: clear.
P1-03: the slot's `Running` moment (A1) and what socket `READY=1` refers to (A10) are guesses; the
tests reference a harness that does not exist yet (A10/B1). P1-04: `ensure_daemon()` has no socket
to `hello` on (A10). P1-05: clear. P1-06: which op is idempotent for `api::ops::TABLE` (A3); the
client timeout mismatch (A8). P1-07: cannot land before the items it tests (A10). P1-08 to P1-10:
clear after B2. P1-11: who writes `api::Client` (A20); `log`/`events` offline behaviour (A9).
P1-12: a five-minute test (A17). P1-13 to P1-18: clear. P2-01 to P2-08: test paths (A13), the
dim ladder and its fixture format (A2, A12), the `startDaemon` rule (A5), `r` in the popup (A19),
right-click with the popup closed (A21). P3 (P3-09 Torrents view): `keymap.json` shape for the
`KeyHint` rows (A12), Esc's last layer (A4), `interval_ms` per topic (A22); otherwise §4.3, §4.6,
§4.9 to §4.11 and Appendix B give everything, including widths that add up. P5 (P5-04 limits):
four names for four numbers and no persisted turtle switch (A14); P5-05: `finishes_in_s` and the
R15 case (A15). Nothing else in P5 needed a guess.

**Milestone truth (check 4).** M0: demo-able (today's app in git, three bugs fixed). M1: demo-able
only with all of P0, P1 and P2 (43 items, 35 S-units on the serial path); it is two milestones,
and P1's end already has a human check ("the owner clicks a magnet link with no window open and
gets a notification, not a terminal"), which is the product's first visible change and the first
moment R5 (lifetime surprise) can be judged (C2). M2 is reachable without P2-03 (the popup
rewrite), which is fine and worth knowing. M3, M4: each demo-able with its predecessors; M4 does not
need M3, only P4-03. M5, M6: lists, not demos; fine for T2/T3. Sizes: 44 S, 45 M, 11 L is plausible
for one developer plus agents if the lanes of §9.4 run in parallel; the serial path is the risk,
not the total (C3 shaves it).

**Risk register (check 5).** Every R1–R17 mitigation is a named item or an explicit acceptance
except R15, whose two-file test existed only in the fixture list (A15). Q1–Q20 each have a default
the plan already implements; Q3 is the one whose default needs the owner's action before the first
commit (C1). One risk is missing: the 35-S-unit serial path to the first demo (C2, C3 are its
mitigations); the §10.1 hunk below adds it.

### C1 (P0). Make Q3 a blocking question and say which branch of it blocks

Analysis: P0-02 is on the path to P0-04, and P0-04 pushes history; whichever answer the owner
gives decides whether 56 MB enters history forever (A23). The plan reads as if P0-02 could be
done by an agent alone ("create `github.com/odcpw/asupersync`"), which needs the owner's GitHub
account. The default in §10.3 is unchanged (the fork); the plan should say the question is asked
first, and what happens on either answer. (The Q3 row edit is in A23; this hunk marks the item.)

```diff
--- PLAN.md (line 2875)
 - *Tests:* the build and test runs, logged to `target/test-logs/asupersync/`; `run_timer_wheel_tests.py`
   green.
-- *Depends on:* none.
+- *Depends on:* none; blocked on the owner's answer to Q3 (the only owner action before P0-04),
+  because the answer decides whether `vendor/asupersync/` enters history in C3.
```

```diff
--- PLAN.md (line 4246)
 | R1 | The uncommitted tree is lost before P0-04 lands | low | critical | Back up first (P0-01); P0-04 is the first change; no destructive git commands (§9.1). |
+| R18 (round 3) | The first user-visible change is at the end of P2, after 35 S-units of serial work through four L items; attention and the owner's patience run out before it | medium | high | A milestone at the end of P1 with its own demo and install (M1a, §9.3, §9.7); P1-03 and P1-19 in parallel (§9.4); the plugin lane starts against P1-06 (§9.2). |
```

### C2 (P1). The end of P1 is a milestone: M1a, "the daemon behind today's UIs"

Analysis: §9.5's P1 human check is already a demo ("clicks a magnet link with no window open and
gets a notification, not a terminal"); P1 also delivers the TUI as a client, idle exit, `events`
and `api`. Installing that build is when the owner first lives with the new lifetime (R5) and when
the compat guard of §3.6 is checked by hand (B2). Naming it costs one line in four places and
gives the beads conversion a ready-work boundary. IDs stay stable (a new milestone, not a renumber).

```diff
--- PLAN.md (line 3488)
-**M1 First demo (v0.2.0).** *Depends on:* P2-03, P2-05, P2-06, P2-07, P1-13, P1-14, P1-15, P1-16,
-P1-17, P1-18. Demo script in §9.7.
+**M1a Daemon behind today's UIs (v0.2.0-pre, installed, not tagged).** *Depends on:* P1-04, P1-11,
+P1-12, P1-13, P1-15, P1-16, P1-17, P1-18. Demo script in §9.7; the receipt records the one-time
+compat check of §3.6.
+
+**M1 First demo (v0.2.0).** *Depends on:* M1a, P1-14, P2-03, P2-05, P2-06, P2-07. Demo script in
+§9.7.
```

```diff
--- PLAN.md (line 4028)
-| M1 | P2-03, P2-05, P2-06, P2-07, P1-13, P1-14, P1-15, P1-16, P1-17, P1-18 | P4-09 |
+| M1a | P1-04, P1-11, P1-12, P1-13, P1-15, P1-16, P1-17, P1-18 | M1 |
+| M1 | M1a, P1-14, P2-03, P2-05, P2-06, P2-07 | P4-09 |
```

```diff
--- PLAN.md (line 4012)
-| P1-11 | P1-04, P1-09 | P1-18, P2-07, P3-07 |
-| P1-12 | P1-08 | P1-15, P1-17, P2-03, P3-02, P3-03 |
-| P1-13 | P1-08 | M1, P5-05, P6-10, P6-13 |
+| P1-11 | P1-04, P1-09 | M1a, P1-18, P2-07, P3-07 |
+| P1-12 | P1-08 | M1a, P1-15, P1-17, P2-03, P3-02, P3-03 |
+| P1-13 | P1-08 | M1a, P5-05, P6-10, P6-13 |
 | P1-14 | P1-08 | M1, P3-10, P3-15, P4-08 |
-| P1-15 | P1-04, P1-10, P1-12 | M1, P4-01 |
-| P1-16 | P1-19, P1-07 | M1, P6-06 |
-| P1-17 | P1-04, P1-12 | M1 |
-| P1-18 | P1-08, P1-11 | M1, P5-05 |
+| P1-15 | P1-04, P1-10, P1-12 | M1a, P4-01 |
+| P1-16 | P1-19, P1-07 | M1a, P6-06 |
+| P1-17 | P1-04, P1-12 | M1a |
+| P1-18 | P1-08, P1-11 | M1a, P5-05 |
```

(After A10, P1-04's row reads `| P1-04 | P1-03 | P1-11, P1-15, P1-17, P2-07 |`; add `M1a` to its
Consumed-by cell as well.)

```diff
--- PLAN.md (line 2826)
-| P1 | `fbittorrent serve`, systemd on demand, API v1 behind today's UIs, agent `events`/`api` | (feeds M1) | |
+| P1 | `fbittorrent serve`, systemd on demand, API v1 behind today's UIs, agent `events`/`api` | M1a "daemon behind today's UIs": the magnet click without a terminal, the TUI as a client | v0.2.0-pre (installed on the owner's machine, not tagged) |
```

```diff
--- PLAN.md (line 4184)
+| M1a | Magnet clicks without terminals; downloads survive closing the TUI; idle exit with `fbittorrent status` showing the countdown; completion and added notifications; `events`/`api` for agents; the old popup still works through the compat socket | v0.2.0-pre (installed) |
 | **M1** | Magnet clicks without terminals; live bar popup in the §4 design; downloads survive closing windows; idle exit; completion and added notifications; the TUI as a client; `events`/`api` for agents | **v0.2.0** |
```

```diff
--- PLAN.md (line 4191)
+**M1a demo script** (record in the M1a receipt): (1) install the P1 build with `scripts/install.sh`
+while the v0.1 TUI is open; run `fbittorrent list --json` (reads answer) and `fbittorrent stop
+<hash>` (refused with "an older fbittorrent app is running; quit it with Q and retry", §3.6); quit
+the old TUI with `Q`. (2) `fbittorrent status --json` → `"running": false`. (3) Click a magnet link
+in the browser → no terminal; the "Added" notification with "Undo" and "Open window"; `fbittorrent
+status --json` shows the download. (4) `fbittorrent tui`, see the torrent, press `q` → the one-time
+notice; `fbittorrent status --json` still shows it downloading. (5) `fbittorrent events --follow
+--json` in another terminal prints `torrent.completed` when it finishes; the "Download complete"
+notification's "Open" starts the player. (6) Wait: `status --json` shows `daemon.idle_exit_in_s`
+counting down; the daemon exits; `systemctl --user is-active fbittorrent` prints `inactive`.
+
 **M1 demo script** (each step has an expected observation; record them in the M1 receipt):
```

### C3 (P2). Let the daemon core and the typed operations proceed in parallel

Analysis: P1-03 (daemon core, L) depends on P1-19 (typed ops, M), but nothing in P1-03's Do or
Accept uses `EngineOp`: the slot, supervisor, signals, `sd_notify` and parking need the split
engine (P1-02) and today's string dispatch, which keeps working. P1-10 (the legacy adapter, which
parses routes into `EngineOp`) is what needs P1-19. Moving the edge lets two agents work the two
items at once and takes one M off the serial path to every milestone.

```diff
--- PLAN.md (line 3122)
   (`vpn.py::headless_park_unpark`, opt-in).
-- *Depends on:* P1-19.
+- *Depends on:* P1-02 (the split engine; today's string dispatch serves `engine.sock` until P1-10), P1-07 (the harness skeleton, A10).
```

```diff
--- PLAN.md (line 3241)
   `legacy::tests::route_parse_roundtrip`; the TUI-as-client cases move to P1-15.
-- *Depends on:* P1-03.
+- *Depends on:* P1-03, P1-19.
```

```diff
--- PLAN.md (line 4002)
-| P1-02 | P1-01 | P1-19 |
-| P1-19 | P1-02 | P1-03, P1-05, P1-16, P5-06, P6-07 |
-| P1-03 | P1-19 | P1-04, P1-06, P1-10 |
+| P1-02 | P1-01 | P1-19, P1-03 |
+| P1-19 | P1-02 | P1-05, P1-10, P1-16, P5-06, P6-07 |
+| P1-03 | P1-02, P1-07 | P1-04, P1-06, P1-10 |
```

```diff
--- PLAN.md (line 4011)
-| P1-10 | P1-03 | P1-15 |
+| P1-10 | P1-03, P1-19 | P1-15 |
```

### C4 (P2). Prose: three phrases that assert instead of state

Light pass only, as the brief asks; the integrator may leave the rest to the prose round.

```diff
--- PLAN.md (line 58)
-4. **The TUI earns its keep as fallback and test bed.** It works over SSH and on a TTY, carries
+4. **The TUI stays because it works where the shell does not.** It works over SSH and on a TTY, carries
```

```diff
--- PLAN.md (line 2828)
-| P3 | Search, RSS, Shows, IMDb in the daemon; dashboard window; poster grid; show quality | M2 "dashboard daily-drivable" | v0.3.0 |
+| P3 | Search, RSS, Shows, IMDb in the daemon; dashboard window; poster grid; show quality | M2 "every step of W1, W2 and W8 works from the dashboard" | v0.3.0 |
```

```diff
--- PLAN.md (line 3703)
-**M2 Dashboard daily-drivable (v0.3.0).** *Depends on:* P3-07, P3-16.
+**M2 Every step of W1, W2 and W8 works from the dashboard (v0.3.0).** *Depends on:* P3-07, P3-16.
```

Other passages a prose round should look at (no hunks here): "the look-and-feel ask" (P2-03 Why),
"a working install story" (P2-07 Why), "Comfort and diagnosis" (P6-03 Why), and §0.2's "Feel" row,
whose "nothing is spawned per tick" is the one measurable clause.

## Section D. Owner questions

1. **Q3, before the first commit:** create `odcpw/asupersync` yourself (the plan's default), or
   accept 56 MB in history once (option B as corrected in A23)? Nothing in P0 can be pushed before
   this is answered.
2. **Popup `q`:** this review makes `q` close the popup and `Q` (or the power icon) quit, so `q`
   means "close this" in every surface (A6). Today's popup stops the daemon on `q`. Acceptable?
3. **A demo checkpoint after P1 (M1a):** worth installing a build whose popup is still the old
   polling one, in exchange for living with the new lifetime (downloads survive windows, idle exit)
   several weeks before the popup rewrite? The compat socket keeps the old popup working.
4. **Look-and-feel gate:** a side-by-side capture of our popup and the Wi-Fi panel that you judge
   (B3), instead of an automated 1 px comparison against the Wi-Fi panel's screenshot?
5. **Dim colour in the TUI:** with one ladder (A2) the TUI's secondary text on your theme becomes
   `#b6b6b6` (the shell's 1.4 convention, 10.6:1) rather than `muted` `#7a7a7a` (4.9:1). Brighter
   and identical to the popup; do you want it that way, or a TUI-only second rung at `muted` when it
   passes 4.5:1 (then the two surfaces differ on your theme, by design)?

## Section E. Verification script output

Script: `scratchpad/r3/verify_hunks.py REVIEW PLAN` (parses every ```diff block in this file; each `-` line and each context line must exist verbatim as a whole line in `PLAN.md`; `+` lines are not checked). Run on 2026-10-18 against `PLAN.md` at 5,075 lines:

```
hunk   1 OK  PLAN.md (line 754): removed 1/1, context 0/0
hunk   2 OK  PLAN.md (line 775): removed 1/1, context 0/0
hunk   3 OK  PLAN.md (line 1484): removed 1/1, context 0/0
hunk   4 OK  PLAN.md (line 2287): removed 1/1, context 0/0
hunk   5 OK  PLAN.md (line 2430): removed 1/1, context 0/0
hunk   6 OK  PLAN.md (line 2443): removed 1/1, context 0/0
hunk   7 OK  PLAN.md (line 3467): removed 2/2, context 0/0
hunk   8 OK  PLAN.md (line 4290): removed 1/1, context 0/0
hunk   9 OK  PLAN.md (line 4298): removed 1/1, context 0/0
hunk  10 OK  PLAN.md (line 4398): removed 0/0, context 1/1
hunk  11 OK  PLAN.md (line 692): removed 2/2, context 2/2
hunk  12 OK  PLAN.md (line 880): removed 1/1, context 2/2
hunk  13 OK  PLAN.md (line 2053): removed 2/2, context 0/0
hunk  14 OK  PLAN.md (line 1760): removed 1/1, context 1/1
hunk  15 OK  PLAN.md (line 1776): removed 2/2, context 0/0
hunk  16 OK  PLAN.md (line 4672): removed 3/3, context 0/0
hunk  17 OK  PLAN.md (line 4708): removed 2/2, context 0/0
hunk  18 OK  PLAN.md (line 410): removed 1/1, context 0/0
hunk  19 OK  PLAN.md (line 2064): removed 3/3, context 0/0
hunk  20 OK  PLAN.md (line 1855): removed 1/1, context 0/0
hunk  21 OK  PLAN.md (line 4300): removed 1/1, context 0/0
hunk  22 OK  PLAN.md (line 4792): removed 1/1, context 0/0
hunk  23 OK  PLAN.md (line 4808): removed 1/1, context 0/0
hunk  24 OK  PLAN.md (line 1748): removed 3/3, context 0/0
hunk  25 OK  PLAN.md (line 665): removed 1/1, context 0/0
hunk  26 OK  PLAN.md (line 699): removed 1/1, context 0/0
hunk  27 OK  PLAN.md (line 3126): removed 1/1, context 0/0
hunk  28 OK  PLAN.md (line 3177): removed 8/8, context 0/0
hunk  29 OK  PLAN.md (line 3191): removed 1/1, context 0/0
hunk  30 OK  PLAN.md (line 3119): removed 3/3, context 0/0
hunk  31 OK  PLAN.md (line 4005): removed 1/1, context 0/0
hunk  32 OK  PLAN.md (line 4007): removed 2/2, context 0/0
hunk  33 OK  PLAN.md (line 4934): removed 0/0, context 1/1
hunk  34 OK  PLAN.md (line 2819): removed 0/0, context 1/1
hunk  35 OK  PLAN.md (line 1700): removed 0/0, context 1/1
hunk  36 OK  PLAN.md (line 1503): removed 2/2, context 0/0
hunk  37 OK  PLAN.md (line 4657): removed 1/1, context 0/0
hunk  38 OK  PLAN.md (line 1419): removed 1/1, context 0/0
hunk  39 OK  PLAN.md (line 2120): removed 1/1, context 0/0
hunk  40 OK  PLAN.md (line 2124): removed 2/2, context 1/1
hunk  41 OK  PLAN.md (line 2132): removed 1/1, context 0/0
hunk  42 OK  PLAN.md (line 2138): removed 1/1, context 0/0
hunk  43 OK  PLAN.md (line 3433): removed 1/1, context 0/0
hunk  44 OK  PLAN.md (line 4970): removed 1/1, context 0/0
hunk  45 OK  PLAN.md (line 2577): removed 1/1, context 0/0
hunk  46 OK  PLAN.md (line 4588): removed 1/1, context 0/0
hunk  47 OK  PLAN.md (line 3895): removed 1/1, context 0/0
hunk  48 OK  PLAN.md (line 4580): removed 1/1, context 0/0
hunk  49 OK  PLAN.md (line 4120): removed 1/1, context 0/0
hunk  50 OK  PLAN.md (line 191): removed 1/1, context 0/0
hunk  51 OK  PLAN.md (line 3283): removed 2/2, context 1/1
hunk  52 OK  PLAN.md (line 3687): removed 1/1, context 0/0
hunk  53 OK  PLAN.md (line 2594): removed 1/1, context 0/0
hunk  54 OK  PLAN.md (line 1579): removed 1/1, context 0/0
hunk  55 OK  PLAN.md (line 3245): removed 1/1, context 0/0
hunk  56 OK  PLAN.md (line 1804): removed 1/1, context 0/0
hunk  57 OK  PLAN.md (line 4513): removed 1/1, context 0/0
hunk  58 OK  PLAN.md (line 2703): removed 4/4, context 0/0
hunk  59 OK  PLAN.md (line 2869): removed 2/2, context 0/0
hunk  60 OK  PLAN.md (line 4313): removed 1/1, context 0/0
hunk  61 OK  PLAN.md (line 4331): removed 1/1, context 0/0
hunk  62 OK  PLAN.md (line 959): removed 1/1, context 0/0
hunk  63 OK  PLAN.md (line 2806): removed 2/2, context 0/0
hunk  64 OK  PLAN.md (line 3043): removed 1/1, context 0/0
hunk  65 OK  PLAN.md (line 3049): removed 1/1, context 1/1
hunk  66 OK  PLAN.md (line 4963): removed 1/1, context 0/0
hunk  67 OK  PLAN.md (line 4166): removed 1/1, context 1/1
hunk  68 OK  PLAN.md (line 3230): removed 12/12, context 0/0
hunk  69 OK  PLAN.md (line 1286): removed 3/3, context 0/0
hunk  70 OK  PLAN.md (line 3260): removed 3/3, context 2/2
hunk  71 OK  PLAN.md (line 4964): removed 1/1, context 0/0
hunk  72 OK  PLAN.md (line 4151): removed 1/1, context 0/0
hunk  73 OK  PLAN.md (line 79): removed 1/1, context 0/0
hunk  74 OK  PLAN.md (line 2141): removed 6/6, context 0/0
hunk  75 OK  PLAN.md (line 3472): removed 4/4, context 0/0
hunk  76 OK  PLAN.md (line 3482): removed 2/2, context 0/0
hunk  77 OK  PLAN.md (line 3452): removed 4/4, context 0/0
hunk  78 OK  PLAN.md (line 3459): removed 1/1, context 1/1
hunk  79 OK  PLAN.md (line 4984): removed 1/1, context 0/0
hunk  80 OK  PLAN.md (line 4172): removed 4/4, context 2/2
hunk  81 OK  PLAN.md (line 4979): removed 3/3, context 0/0
hunk  82 OK  PLAN.md (line 3858): removed 2/2, context 1/1
hunk  83 OK  PLAN.md (line 3151): removed 2/2, context 0/0
hunk  84 OK  PLAN.md (line 4982): removed 1/1, context 0/0
hunk  85 OK  PLAN.md (line 2734): removed 1/1, context 0/0
hunk  86 OK  PLAN.md (line 1061): removed 1/1, context 0/0
hunk  87 OK  PLAN.md (line 1072): removed 2/2, context 0/0
hunk  88 OK  PLAN.md (line 902): removed 2/2, context 0/0
hunk  89 OK  PLAN.md (line 1014): removed 1/1, context 0/0
hunk  90 OK  PLAN.md (line 4405): removed 1/1, context 0/0
hunk  91 OK  PLAN.md (line 4620): removed 1/1, context 0/0
hunk  92 OK  PLAN.md (line 3147): removed 1/1, context 0/0
hunk  93 OK  PLAN.md (line 3149): removed 1/1, context 1/1
hunk  94 OK  PLAN.md (line 3361): removed 2/2, context 0/0
hunk  95 OK  PLAN.md (line 3375): removed 1/1, context 0/0
hunk  96 OK  PLAN.md (line 4385): removed 0/0, context 1/1
hunk  97 OK  PLAN.md (line 3189): removed 2/2, context 0/0
hunk  98 OK  PLAN.md (line 3810): removed 1/1, context 0/0
hunk  99 OK  PLAN.md (line 3978): removed 1/1, context 0/0
hunk 100 OK  PLAN.md (line 2655): removed 6/6, context 1/1
hunk 101 OK  PLAN.md (line 536): removed 1/1, context 0/0
hunk 102 OK  PLAN.md (line 4044): removed 1/1, context 0/0
hunk 103 OK  PLAN.md (line 4081): removed 1/1, context 0/0
hunk 104 OK  PLAN.md (line 4087): removed 1/1, context 0/0
hunk 105 OK  PLAN.md (line 4585): removed 1/1, context 0/0
hunk 106 OK  PLAN.md (line 1024): removed 1/1, context 0/0
hunk 107 OK  PLAN.md (line 3304): removed 1/1, context 0/0
hunk 108 OK  PLAN.md (line 3436): removed 2/2, context 0/0
hunk 109 OK  PLAN.md (line 2135): removed 1/1, context 1/1
hunk 110 OK  PLAN.md (line 1083): removed 10/10, context 1/1
hunk 111 OK  PLAN.md (line 2439): removed 1/1, context 0/0
hunk 112 OK  PLAN.md (line 93): removed 4/4, context 0/0
hunk 113 OK  PLAN.md (line 2875): removed 1/1, context 2/2
hunk 114 OK  PLAN.md (line 4246): removed 0/0, context 1/1
hunk 115 OK  PLAN.md (line 3488): removed 2/2, context 0/0
hunk 116 OK  PLAN.md (line 4028): removed 1/1, context 0/0
hunk 117 OK  PLAN.md (line 4012): removed 7/7, context 1/1
hunk 118 OK  PLAN.md (line 2826): removed 1/1, context 0/0
hunk 119 OK  PLAN.md (line 4184): removed 0/0, context 1/1
hunk 120 OK  PLAN.md (line 4191): removed 0/0, context 1/1
hunk 121 OK  PLAN.md (line 3122): removed 1/1, context 1/1
hunk 122 OK  PLAN.md (line 3241): removed 1/1, context 1/1
hunk 123 OK  PLAN.md (line 4002): removed 3/3, context 0/0
hunk 124 OK  PLAN.md (line 4011): removed 1/1, context 0/0
hunk 125 OK  PLAN.md (line 58): removed 1/1, context 0/0
hunk 126 OK  PLAN.md (line 2828): removed 1/1, context 0/0
hunk 127 OK  PLAN.md (line 3703): removed 1/1, context 0/0

hunks: 127; removed lines: 213; context lines: 32; not found: 0
```

The script source, so the integrator can re-run it after applying hunks:

```python
#!/usr/bin/env python3
"""Check that every removed ('-') and context (' ') line of every ```diff hunk in the review
exists verbatim as a whole line in PLAN.md. Usage: verify_hunks.py REVIEW PLAN"""
import sys, re
review = open(sys.argv[1], encoding="utf-8").read().split("\n")
plan_lines = open(sys.argv[2], encoding="utf-8").read().split("\n")
plan = {}
for i, l in enumerate(plan_lines, 1):
    plan.setdefault(l, []).append(i)
in_diff, hunk_no, header = False, 0, ""
removed = context = missing = 0
per_hunk = []
for l in review:
    if l.startswith("```diff"):
        in_diff, hunk_no = True, hunk_no + 1
        header, r_ok, r_bad, c_ok, c_bad = "", 0, [], 0, []
        continue
    if in_diff and l.startswith("```"):
        in_diff = False
        per_hunk.append((hunk_no, header, r_ok, r_bad, c_ok, c_bad))
        continue
    if not in_diff:
        continue
    if l.startswith("--- PLAN.md"):
        header = l[4:]
        continue
    if l.startswith("+"):
        continue
    if l.startswith("-"):
        body = l[1:]
        removed += 1
        if body in plan: r_ok += 1
        else: r_bad.append(body); missing += 1
    elif l.startswith(" "):
        body = l[1:]
        context += 1
        if body in plan: c_ok += 1
        else: c_bad.append(body); missing += 1
    elif l == "":
        # an empty context line (blank line in the plan) is always present
        context += 1; c_ok += 1
for n, h, r_ok, r_bad, c_ok, c_bad in per_hunk:
    status = "OK " if not r_bad and not c_bad else "MISSING"
    print(f"hunk {n:3d} {status} {h}: removed {r_ok}/{r_ok+len(r_bad)}, context {c_ok}/{c_ok+len(c_bad)}")
    for b in r_bad: print(f"      removed line not found: {b[:110]!r}")
    for b in c_bad: print(f"      context line not found: {b[:110]!r}")
print(f"\nhunks: {len(per_hunk)}; removed lines: {removed}; context lines: {context}; not found: {missing}")
sys.exit(1 if missing else 0)
```
