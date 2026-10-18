# Round 2 review: product fit, look and feel, interaction model, roadmap

Reviewer lens: product fit for this owner, the Quickshell popup and dashboard design, the TUI
design, the keyboard model, UX heuristics, and the feature roadmap. Architecture, API and engine
hygiene are the sibling review's job; they appear here only where a UI decision depends on them.

What was read: `PLAN.md` (4,676 lines, round 1) end to end, `review-log.md`, `README.md`,
`reality-check.md` (the 15 UX defects and the 36-feature inventory), `frontend-options.md` §2, §5,
§8, `omatorrent-research.md` §3, §5, §6, §8, every capture under `screens/` (26 text, 8 ANSI, 2
PNG), `docs/tui-ux-review.md`, the live plugin (`Panel.qml`, `Service.qml`, `Model.js`,
`manifest.json`), `src/tui/{mod,theme,poster}.rs` (the cited regions), the installed shell
(`Commons/Style.qml`, `Color.qml`, `Border.qml`; `Ui/Panel`, `KeyboardPanel`, `PanelHero`,
`PanelSectionHeader`, `PanelKeyCatcher`, `FilterInput`, `ConfirmDialog`, `PanelActionButton`,
`CursorSurface`, `Dropdown`, `ToggleSwitch`; `plugins/panels/network`, `bluetooth`,
`dev-gallery`, `bar/Bar.qml`, `services/PluginShellApi.qml`, `PluginRegistry.qml`, `shell.qml`),
the sibling plugins, the research clones (omaqbt `Panel`/`Client`/`TorrentTable`/`WmFocus`/README,
Sprout `Panel`/`BarWidget`/`Dashboard`/`BRAND.md`/`PRODUCT.md`, Fragments, Transmission,
qBittorrent at the cited lines), the ftui 0.6 crate sources (`table.rs`, `virtualized.rs`,
`scrollbar.rs`, `toast.rs`, `notification_queue.rs`, `log_viewer.rs`, `sparkline.rs`, `badge.rs`,
`tabs.rs`, `ftui-extras` `image.rs`, `canvas.rs`), and `omarchy-src/agents/skills/shell-dev.md`.

Three things were measured rather than read: every mockup line's display width, the WCAG
contrast of `foreground`, `Qt.darker(foreground, 1.4)`, `muted`, `dark_foreground` and `accent`
for all 23 installed themes plus the owner's, and the dashboard column arithmetic at each width
tier. The numbers are quoted where they matter.

Nothing under `src/`, `plugin/`, `vendor/`, `Cargo.*` or `PLAN.md` was modified.

---

## Verdict

Round 1 is a real design specification, not a feature list: nearly every token, component and
prior-art citation in §4 and §5 checks out against the installed shell and the clones, the row
anatomy and state tables are concrete, and the 17 UX defects each have an item and a test. If the
plan were implemented exactly as written, the owner would get a popup that looks first-party and
a TUI that is finally legible at 90 columns. They would **not yet** feel that the dashboard and
the headline streaming feature reached OmaTorrent's level, for five reasons that are all fixable
in this round:

1. **The dashboard geometry does not add up.** With the specified pane widths (sidebar 210,
   details 380, `panelGap` 14, `PaneFrame` padding 8) and column widths (848 px without NAME),
   the list pane at the plan's own "≥ 1400" tier is about 734 px wide, so NAME is negative; at the
   1280 px default window it is still negative after the tier's hidden columns. The first thing
   the owner sees on opening the window would be a broken table (A1, B1).
2. **Two acceptance criteria are impossible on installed themes.** The shell's own
   `Qt.darker(fg, 1.4)` dim is 3.8:1 on everforest and 4.3:1 on gruvbox and tokyo-night, so
   P2-08's "fails below 4.5:1 for dim text on every theme" turns the lint red on day one;
   `Color.muted`, adopted for "—", table headers and empty-state body, is below 4.5:1 on 20 of 23
   themes; the TUI's "text reaches 7:1" fails on rose-pine (6.7:1). Either the gates get quietly
   relaxed or the design does not actually hold on the light and warm themes the plan promises to
   capture (A2, A3, B2).
3. **The idle-exit policy, combined with the hidden bar icon, produces a surface that vanishes
   under the owner's cursor.** The countdown keeps running while the popup is open, and
   `visible: svc.online` hides the icon the popup is anchored to. "Ready to watch" on the icon has
   no defined acknowledgement. These are new defects the plan would introduce (A6, B9).
4. **The "shared state vocabulary" is not shared.** §4.3.4 says one table feeds the popup, the
   dashboard and the TUI, but §5.5.3 uses different glyphs and five-letter words, and `…` means
   "metadata" on Torrents while it already means "queued/waiting" on Shows (`screens/03-help-page3`).
   That is exactly the R4 drift the plan says it fears (A4).
5. **Ready-to-watch cries wolf.** 32 MiB or 3 % plus the last piece is 30 seconds of a 1080p
   file; on any link slower than the bitrate the player stalls a minute later. The feature needs
   a "will not stall at the current rate" criterion next to the threshold, or the owner learns to
   ignore the notification (B3).

Secondary but still worth fixing before beads: three key overloads the plan admits (`u`, `Q`,
`s`), a Space exception that the platform does not actually force, a popup quick-add that commits
with one key and no preview, no disk-full behaviour anywhere, and acceptance lines that cannot be
checked from a capture ("responsive", "README reviewed", "indistinguishable").

---

## Section A. Defects

Each entry: location, evidence, fix as a unified-diff hunk against `PLAN.md`. Line numbers are
round-1 line numbers; hunks quote enough context to apply by content.

### A1. The dashboard columns cannot fit at the plan's own width tiers

**Location:** §4.2.2 (lines 1435-1439), §4.3.2 (1535-1537, 1543-1546), §4.3.3 (1561-1571), §4.6
(1720, 1761).

**Evidence (arithmetic from the plan's numbers):**

- Chrome with both side panes docked: sidebar `Style.space(210)` + details `Style.space(380)` +
  2 × `panelGap` (14) + 3 panes × 2 × `PaneFrame` padding `lg` (8) = **666 px**.
- Fixed columns without NAME (§4.3.2): 26 + 110 + 84 + 150 + 96 + 90 + 70 + 70 + 62 + 90 = **848 px**.
- Tier "≥ 1400": list pane = 1400 − 666 = 734 px → NAME = 734 − 848 = **−114 px**.
- Tier "1100 to 1399" at the 1280 px `implicitWidth`, RATIO and ADDED hidden (848 − 152 = 696):
  list = 614 px → NAME = **−82 px**. At 1100 px: **−262 px**.
- Only the "900 to 1099" tier (no sidebar, sheet details, SIZE + ETA) fits: 884 − 536 = 348 px.
- The ≥ 1400 tier first fits all columns with NAME ≥ 80 at about **1,600 px**, and even then NAME
  gets 86 px, which is six characters.
- The parenthetical "a 1280 px window holds more" (line 1761) is therefore false.

omaqbt, whose widths the plan borrows, sizes its window 1600×900 and has **no** STATE, PEERS or
ADDED column (`omaqbt TorrentTable.qml:48-56`; `Client.qml:736,803`). Adding three columns to
omaqbt's set while keeping omaqbt's pane widths is what breaks it.

**Fix:** narrower side panes, tighter numeric columns (the mockup already uses compact "1.1G",
"3.1M", "12m"), a NAME floor, and tiers recomputed so each one is proven to fit. Default window
1600×900 like omaqbt so the first open lands in the full tier; the owner's tiled 960 px window
lands in the chip-row tier, which is designed, not degraded.

```diff
--- a/docs/plan-2026-10-18/PLAN.md
+++ b/docs/plan-2026-10-18/PLAN.md
@@ -1435,5 +1435,6 @@
-- The dashboard is three panes on a horizontal axis: sidebar `Style.space(210)`, list (fill),
-  details `Style.space(380)` (omaqbt's measured widths, `omaqbt Client.qml:736,763,803`). Panes
-  are separated by `Style.spacing.panelGap` and framed by `PaneFrame` (a `BorderSurface` with
-  `Border.controlSpec("normal")`, padding `Style.spacing.lg`). Each pane has a title row of
-  `Style.space(30)` and a body.
+- The dashboard is three panes on a horizontal axis: sidebar `Style.space(200)`, list (fill),
+  details `Style.space(360)` (omaqbt uses 210/380 for a table without STATE, PEERS and ADDED,
+  `omaqbt Client.qml:736,763,803`; ours carries three more columns, so the side panes give back
+  30 px). Panes are separated by `Style.spacing.panelGap` and framed by `PaneFrame` (a
+  `BorderSurface` with `Border.controlSpec("normal")`, padding `Style.spacing.lg`). Each pane has
+  a title row of `Style.space(30)` and a body. Chrome with both side panes docked is therefore
+  200 + 360 + 2 × 14 + 3 × 16 = 636 px; with the details as a sheet 200 + 14 + 32 = 246 px; with
+  the sidebar collapsed to chips 16 px. §4.3.3's tiers are computed from these three numbers.
```

```diff
@@ -1535,3 +1535,3 @@
- glyph │ STATE        │ NAME (fill)                        │ SIZE  │ PROGRESS        │  ↓   │  ↑  │ ETA │ PEERS │ RATIO │ ADDED
-  26   │ 110          │ ≥ 80                                │ 84    │ 150             │ 96   │ 90  │ 70  │ 70    │ 62    │ 90    (Style.space)
+ glyph │ STATE        │ NAME (fill)                        │ SIZE  │ PROGRESS        │  ↓   │  ↑  │ ETA │ PEERS │ RATIO │ ADDED
+  26   │ 110          │ ≥ 210                               │ 76    │ 120             │ 80   │ 80  │ 64  │ 56    │ 56    │ 80    (Style.space)
```

```diff
@@ -1543,4 +1543,7 @@
-- Widths are omaqbt's measured set (`omaqbt TorrentTable.qml:48-56`) plus three columns: STATE
-  (glyph and word, never hidden), PEERS, ADDED. Row height `Style.spacing.popupRowHeight`, header
-  `Style.space(30)`, cell padding `Style.space(10)`, numerics right-aligned, name left-aligned
-  with `ElideRight` and the full name in the details pane and a tooltip.
+- Widths start from omaqbt's measured set (`omaqbt TorrentTable.qml:48-56`), tightened to the
+  compact number formats of §4.2.3 ("1.1G", "3.1M", "12m", "Oct 12"), plus three columns: STATE
+  (glyph and word, never hidden), PEERS, ADDED. STATE is 110 because "Downloading" is 11 cells of
+  `Style.font.subtitle` (about 86 px at base size 12) plus 2 × `Style.space(10)` padding; the
+  fixed columns without NAME sum to 748 px. Row height `Style.spacing.popupRowHeight`, header
+  `Style.space(30)`, cell padding `Style.space(10)`, numerics right-aligned, name left-aligned
+  with `ElideRight` and the full name in the details pane and a tooltip. NAME never goes below
+  `Style.space(210)`; a tier is valid only if NAME ≥ 210 at the tier's narrowest width.
```

```diff
@@ -1561,11 +1561,15 @@
-The dashboard adapts by window width (`FloatingWindow.width`; `minimumSize` 900×560).
+The dashboard adapts by window width (`FloatingWindow.width`; `minimumSize` 900×560;
+`implicitWidth/Height` 1600×900, omaqbt's size, so a floating first open lands in the full tier).
+Each tier below is checked: list width = window − chrome (§4.2.2), NAME = list − fixed columns.
 
-| Window width | Sidebar | Details | Table columns (in addition to glyph, STATE, NAME, PROGRESS, ↓, which never hide) |
-|---|---|---|---|
-| ≥ 1400 | docked (210) | docked (380) | SIZE, ↑, ETA, PEERS, RATIO, ADDED |
-| 1100 to 1399 | docked | docked | SIZE, ↑, ETA, PEERS (RATIO and ADDED hidden) |
-| 900 to 1099 | collapsed to a chip row above the table ("All 12 · Downloading 2 · …") | opens as a sheet over the list (Enter), closes with Esc | SIZE, ETA (↑ and PEERS hidden) |
+| Window width | Sidebar | Details | Table columns (in addition to glyph, STATE, NAME, PROGRESS, ↓, which never hide) | List width → NAME |
+|---|---|---|---|---|
+| ≥ 1600 | docked (200) | docked (360) | SIZE, ↑, ETA, PEERS, RATIO, ADDED | ≥ 964 → ≥ 216 |
+| 1400 to 1599 | docked | docked | SIZE, ETA, PEERS (↑, RATIO, ADDED hidden) | 764–963 → 232–431 |
+| 1100 to 1399 | docked | opens as a sheet over the list (Enter), closes with Esc | SIZE, ↑, ETA, PEERS (RATIO and ADDED hidden) | 854–1153 → 242–541 |
+| 900 to 1099 | collapsed to a chip row above the table ("All 12 · Downloading 2 · …") | sheet | SIZE, ↑, ETA, PEERS (RATIO and ADDED hidden) | 884–1083 → 272–471 |
 
-Drop order as width shrinks: ADDED → RATIO → ↑ → PEERS → ETA → SIZE. Hidden columns' values
+Drop order as width shrinks: ADDED → RATIO → ↑ → PEERS → ETA → SIZE; the details pane undocks
+before PEERS is dropped, because the pane costs 376 px and PEERS 56. Hidden columns' values
```

```diff
@@ -1719,3 +1719,3 @@
-- `Dashboard.qml` is the `panel` entry point. `open(payloadJson)` creates or shows a
-  `FloatingWindow` titled `fbittorrent` (`minimumSize` 900×560, `implicitWidth/Height` 1280×800;
+- `Dashboard.qml` is the `panel` entry point. `open(payloadJson)` creates or shows a
+  `FloatingWindow` titled `fbittorrent` (`minimumSize` 900×560, `implicitWidth/Height` 1600×900;
```

```diff
@@ -1761,1 +1761,2 @@
-(The mockup is about 120 text columns wide; a 1280 px window holds more.)
+(The mockup is about 120 text columns wide and shows the ≥ 1600 px tier; at the owner's tiled
+width of about 960 px the window is in the chip-row tier of Appendix C.)
```

The P3-09 acceptance "resizing the window through 1399 and 1099 px hides the documented columns"
becomes "through 1599, 1399 and 1099 px" and gains "NAME is never narrower than 210 px at any
width ≥ 900 (asserted by `qs-dashboard.sh::width_tiers` reading the header cell widths)".

### A2. The QML contrast gate fails on three installed themes, and `Color.muted` fails on twenty

**Location:** §4.2.1 token table rows "Dim text" and "Unknown marker" (1407-1408), the rule
"Dim is 1.4, not 1.55" (1418-1423), §4.2.3 rows "Table header", "Table cell", "Empty-state body"
(1453, 1456, 1459), §4.11 (omaqbt state copy), P2-08 acceptance (3220-3222).

**Evidence (WCAG ratios computed from `/usr/share/omarchy/themes/*/colors.toml`):**

| Theme | fg:bg | `Qt.darker(fg,1.4)`:bg | `muted`:bg | `accent`:bg |
|---|---|---|---|---|
| everforest | 7.4 | **3.8** | 1.6 | 5.7 |
| gruvbox | 8.2 | **4.3** | 2.3 | 5.9 |
| tokyo-night | 8.1 | **4.3** | 1.9 | 6.8 |
| catppuccin-latte | 7.1 | 10.2 | **1.9** | **4.3** |
| nord | 9.2 | 4.7 | **1.7** | 4.6 |
| matte-black | 10.1 | 5.3 | **1.5** | 7.4 |
| rose-pine | **6.7** | 9.8 | **1.5** | **3.1** |
| miasma | 8.8 | 4.6 | 2.8 | **3.9** |
| vantablack (owner) | 21.0 | 10.4 | 4.9 | 6.3 |

`muted` is below 4.5:1 on every theme except ethereal, starship and the owner's; it is a track
and border colour in the shell (`Commons/Color.qml:168` falls back to `color8`), not a text
colour. The plan nevertheless adopts it for "—" and "∞" (omaqbt `TorrentTable.qml:65-67`), the
table header (omaqbt `:134`), the empty-state body (omaqbt `:314`) and the loading line. Those are
10-13 px text. P2-08's acceptance says the lint fails when dim or primary text is below 4.5:1
"for every theme"; with the fixed 1.4 factor the lint is red on three themes the plan cannot
change, and the muted text is never checked at all.

The TUI already has the right rule (§5.5.1: try `muted`, then `dark_foreground`, then blend until
4.5:1). The QML side should use the same rule, so that "one rule, two implementations, one
fixture" holds for colour exactly as the plan wants it for numbers (§4.2.3) and state words.

**Fix:**

```diff
@@ -1407,2 +1407,2 @@
-| Dim text | `Qt.darker(foreground, 1.4)` (the shell's own convention) | | `Ui/PanelHero.qml:23`, `Ui/PanelSectionHeader.qml:20` |
-| Unknown marker | `Color.muted` for "—" and "∞" | | omaqbt `TorrentTable.qml:65-67` (adopted) |
+| Dim text | `Model.dim(foreground, background)`: the first of `Qt.darker(foreground, 1.4)` (the shell's convention), `Color.muted`, and a blend of `foreground` toward `background` at 0.75 then 0.85 that reaches 4.5:1; `foreground` itself as the last resort | theme | `Ui/PanelHero.qml:23`, `Ui/PanelSectionHeader.qml:20`; the same ladder as the TUI's §5.5.1 |
+| Unknown marker | the dim colour for "—" (never "∞", §4.2.3) | | omaqbt uses `Color.muted` (`TorrentTable.qml:65-67`); not adopted, because `muted` is a track colour that reaches 4.5:1 on 3 of 23 installed themes |
+| Accent text | `Color.accent` for the ready badge, key letters, the sorted header and the active filter only when `accent` reaches 4.5:1 against `background`; otherwise bold `foreground` (the TUI's §5.5.1 rule) | theme | computed: `catppuccin-latte` 4.3:1, `miasma` 3.9:1, `rose-pine` 3.1:1 fail |
```

```diff
@@ -1418,6 +1418,10 @@
-- **Dim is 1.4, not 1.55.** Today's popup uses `Qt.darker(foreground, 1.55)` (`Panel.qml:18`);
-  first-party panels use 1.4. On the owner's theme 1.4 gives `#b6b6b6` on black (about 10.6:1);
-  on `catppuccin-latte` and `flexoki-light` it darkens an already dark foreground, which stays
-  readable. P2-08 adds a test that computes the contrast of `foreground`, the dim and `muted`
-  against `background` for every theme under `/usr/share/omarchy/themes/*/colors.toml` and
-  fails below 4.5:1 for primary and dim text.
+- **Dim starts at 1.4 and is computed, not fixed.** Today's popup uses `Qt.darker(foreground,
+  1.55)` (`Panel.qml:18`); first-party panels use 1.4. On the owner's theme 1.4 gives `#b6b6b6`
+  on black (about 10.6:1) and on `catppuccin-latte` 10.2:1, but on `everforest` it is 3.8:1 and on
+  `gruvbox` and `tokyo-night` 4.3:1 (computed from their `colors.toml`), so a fixed factor cannot
+  pass a 4.5:1 gate. `Model.dim(fg, bg)` therefore walks the ladder in the token table and the
+  TUI's `palette.rs` walks the same ladder (§5.5.1); `qs-format.sh` and `palette::tests` share one
+  fixture of theme files and expected ratios. P2-08's `contrast.py` computes, for every theme under
+  `/usr/share/omarchy/themes/*/colors.toml` and the owner's, the ratio of `foreground`, the
+  computed dim and `accent` against `background`, and fails only when the computed dim is below
+  4.5:1 or primary text is below 4.5:1 (a theme whose own foreground fails is reported, not fixed,
+  because the plugin cannot change it; `rose-pine` is 6.7:1 today). `muted` is reported for
+  information and never used for text.
```

```diff
@@ -1453,1 +1453,1 @@
-| Table header ("NAME", "SIZE") | `Style.font.bodySmall` | uppercase, letterSpacing 0.08 em | muted; sorted column foreground (omaqbt `TorrentTable.qml:120-135`) |
+| Table header ("NAME", "SIZE") | `Style.font.bodySmall` | uppercase, letterSpacing 0.08 em | dim; sorted column foreground (omaqbt `TorrentTable.qml:120-135` uses `muted`; see §4.2.1 why not) |
@@ -1456,1 +1456,1 @@
-| Table cell | `Style.font.subtitle` | regular, numerics right-aligned | foreground; "—" muted |
+| Table cell | `Style.font.subtitle` | regular, numerics right-aligned | foreground; "—" dim |
@@ -1459,1 +1459,1 @@
-| Empty-state title / body | `Style.font.heading` / `body` | regular | foreground / muted |
+| Empty-state title / body | `Style.font.heading` / `body` | regular | foreground / dim |
```

P2-08's acceptance line "`contrast.py` passes for every installed theme (or lists the failing
ones with ratios, and the lint fails)" stays true under the computed dim; add "and
`qs-format.sh::dim_ladder` asserts `Model.dim` returns the same hex as `palette.rs` for every
fixture theme".

### A3. The TUI's "text reaches 7:1" acceptance fails on an installed theme

**Location:** §5.5.1 last paragraph (2112-2113); P4-05 acceptance (3474-3475).

**Evidence:** `rose-pine` foreground `#e0def4` on background `#191724` is 6.7:1. The TUI reads the
theme's foreground as is ("text: as is", §5.5.1) and cannot raise it; the test would fail forever
on that theme, which is in `/usr/share/omarchy/themes/`.

**Fix:**

```diff
@@ -2112,2 +2112,4 @@
-A contrast test (P4-05) loads every `colors.toml` under `/usr/share/omarchy/themes/` plus the
-owner's current one and asserts the computed dim reaches 4.5:1 and text reaches 7:1 on each.
+A contrast test (P4-05) loads every `colors.toml` under `/usr/share/omarchy/themes/` plus the
+owner's current one and asserts the computed dim reaches 4.5:1 and `foreground` reaches 4.5:1 on
+each (`rose-pine` is 6.7:1, so 7:1 is not a gate the TUI can meet; the test prints the ratio so
+a theme below 7:1 is visible in the log). The same theme fixture feeds `contrast.py` (P2-08).
```

### A4. The state vocabulary is three tables, not one, and `…` already means something else

**Location:** §4.3.4 (1575-1590), §5.5.3 (2125-2144), `screens/03-help-page3.txt` line 217,
[UX] "new, unread, added before and complete".

**Evidence:** §4.3.4 says "`Model.js` maps the normalized state to exactly one glyph and one
word; the same table feeds the popup, the table, the TUI's state column (§5.6)". §5.5.3 then
defines a different glyph set (`↓ ~ … ⟳ ⋯ ‖ ✓ ↑ ⏚ ! ▶`) and five-letter words (`Down`, `Stall`,
`Meta`, `Check`, `Queue`, `Pause`), so the TUI column cannot be fed by §4.3.4's table. §5.5.3 also
claims "these reuse the vocabulary the Shows screen already uses", but Shows uses `…` for
"queued/waiting" (help page 3: "↓ downloading · … queued/waiting · ‖ paused · ✓ complete · + added
earlier") while §5.5.3 assigns `…` to `metadata` and `⋯` to `queued`. `…` and `⋯` are also near
identical at a glance, which defeats the glyph-plus-word rule on a 9-cell column.

**Fix:** one table with four columns (`glyph`, `word`, `short`, Nerd Font glyph for QML), the TUI
uses `short`, Shows and RSS transfer symbols come from the same rows, and `…` keeps its shipped
meaning (queued/waiting).

```diff
@@ -1575,3 +1575,6 @@
-`Model.js` maps the normalized `state` (§3.5) to exactly one glyph and one word; the same table
-feeds the popup, the table, the TUI's state column (§5.6) and `fbittorrent list`'s human output.
+`Model.js` and `src/tui/format.rs` carry the same table (one fixture, `contracts/ui/states.json`,
+checked by `qs-format.sh::states` and `format::tests::states`): per normalized `state` (§3.5) one
+Nerd Font glyph (QML), one text glyph (TUI and CLI, width 1), one word, and one `short` word of at
+most five cells for the TUI's 9-cell state column (§5.5.3). The popup and the table use glyph +
+word; the TUI column uses text glyph + `short`; `fbittorrent list` uses the word. The Shows and
+RSS transfer symbols are the same rows (`queued` is `…`, as the Shows screen already shows).
 
-| `state` | Glyph | Word | Secondary (when present) |
-|---|---|---|---|
-| `metadata` | dots + spinner | Metadata | "0:12" elapsed |
-| `checking` | sync | Checking | percent checked |
-| `downloading` | `󰇚` | Downloading | ↓ rate · ETA · peers |
-| `stalled` | timer-sand | Stalled | "no peers" or "no data for 2 min" |
-| `queued` | clock-outline | Queued | "3 of 3 slots busy" |
-| `paused` | `󰏤` | Paused | percent |
-| `done` | `󰄬` | Done | size |
-| `seeding` | `󰕒` | Seeding | ↑ rate · ratio |
-| `waiting_vpn` | `󰦝` | Waiting for VPN | interface name |
-| `error` | `󰀦` | Error | the message |
-| pending verb | the state's glyph at 0.55 opacity | "pausing…", "resuming…", "removing…", "adding…", "checking…" | |
+| `state` | QML glyph | Text glyph | Word | `short` | Secondary (when present) |
+|---|---|---|---|---|---|
+| `metadata` | dots-horizontal + spinner | `◌` | Metadata | Meta | "0:12" elapsed |
+| `checking` | sync | `⟳` | Checking | Check | percent checked |
+| `downloading` | `󰇚` | `↓` | Downloading | Down | ↓ rate · ETA · peers |
+| `stalled` | timer-sand | `~` | Stalled | Stall | "no peers" or "no data for 2 min" |
+| `queued` | clock-outline | `…` | Queued | Queue | "3 of 3 slots busy" |
+| `paused` | `󰏤` | `‖` | Paused | Pause | percent |
+| `done` | `󰄬` | `✓` | Done | Done | size |
+| `seeding` | `󰕒` | `↑` | Seeding | Seed | ↑ rate · ratio |
+| `waiting_vpn` | `󰦝` | `⏚` | Waiting for VPN | VPN | interface name |
+| `error` | `󰀦` | `!` | Error | Error | the message |
+| ready (P5-05, overlays `downloading`) | play-circle-outline | `▶` | Ready to watch | Ready | "4 min buffered" |
+| added earlier (Shows, RSS) | plus | `+` | Added before | n/a | |
+| pending verb | the state's glyph at 0.55 opacity | the state's glyph | "pausing…", "resuming…", "removing…", "adding…", "checking…" | | |
+
+`◌` (U+25CC) replaces `…` for metadata because `…` already means queued/waiting on Shows; the
+width test of §5.5.3 and the glyph render test of P2-04 cover it, and `?` is the fallback if the
+font lacks it.
```

```diff
@@ -2129,13 +2129,3 @@
-| `state` | Cell | Note |
-|---|---|---|
-| `downloading` | `↓ Down` | |
-| `stalled` | `~ Stall` | |
-| `metadata` | `… Meta` | spinner in the footer while metadata resolves |
-| `checking` | `⟳ Check` | |
-| `queued` | `⋯ Queue` | |
-| `paused` | `‖ Pause` | |
-| `done` | `✓ Done` | |
-| `seeding` | `↑ Seed` | |
-| `waiting_vpn` | `⏚ VPN` | |
-| `error` | `! Error` | bold; the message in the details strip |
-| ready to watch (P5-05) | `▶ Ready` | replaces `↓ Down` while the torrent is both downloading and playable |
+The cell is the "Text glyph" and `short` columns of §4.3.4's table (`◌ Meta`, `⟳ Check`,
+`↓ Down`, `~ Stall`, `… Queue`, `‖ Pause`, `✓ Done`, `↑ Seed`, `⏚ VPN`, `! Error` in bold with the
+message in the details strip, `▶ Ready` replacing `↓ Down` while the torrent is both downloading
+and playable). A spinner runs in the footer while metadata resolves.
```

### A5. The dashboard window never names its surface tokens

**Location:** §4.6 first bullet (1719-1727); §4.2.1 lists `Color.popups.*` but §4 never says which
surface the `FloatingWindow` is.

**Evidence:** the only first-party `FloatingWindow` paints `color: root.background` where
`background` is `Commons.Color.background` (`plugins/dev-gallery/GalleryPanel.qml:71,281`), text
in `Commons.Color.foreground`, and uses `PanelKeyCatcher` inside a `FocusScope`. The dashboard's
sheets and dialogs are popup surfaces (`ConfirmDialog` uses `Commons.Color.background` as card and
`Util.alpha(background, 0.7)` as scrim, `Ui/ConfirmDialog.qml:13-15`). Without this line an
implementer will pick `Color.popups.background` for the window, which on themes with a
translucent popup alpha renders a see-through application window.

**Fix:**

```diff
@@ -1727,1 +1727,6 @@
   (`plugins/dev-gallery/GalleryPanel.qml:286-289`).
+- Surfaces: the window paints `Commons.Color.background` with text in `Commons.Color.foreground`
+  (the dev gallery's choice, `GalleryPanel.qml:71,281`); pane frames use
+  `Border.controlSpec("normal")`; the add sheet, the details sheet and the help overlay are cards
+  on a `Util.alpha(Commons.Color.background, 0.7)` scrim with `Border.surfaceSpec("popups",
+  "border", …)`, the same recipe as `Ui/ConfirmDialog.qml:13-15,52-58`; tooltips use
+  `PanelToolTip`. `Color.popups.background` is never used for the window itself.
```

### A6. Idle exit and the hidden icon pull the surface out from under the owner

**Location:** §3.2.10 (805, "The bar never counts"), §4.4 rule 8 (1616-1618), §4.5 visibility
(1670-1675), §4.5 icon table row "Ready to watch (any row, until opened or dismissed)" (1659).

**Evidence:**

- `visible: svc.online || showWhenStopped` with the default `false`: when the idle countdown
  reaches zero while the popup is open (the popup does not count as interactive, §3.2.10), the
  daemon sends `bye{idle}`, `online` flips to false, the `BarIconButton` the `KeyboardPanel` is
  anchored to disappears, and the popup closes under the cursor. The owner sees the app vanish
  while looking at it. Nothing in §4.11 covers "the daemon exited while the popup was open".
- The popup shows "stops in 4 min" (§4.5) while the owner is reading it; a glance surface that
  announces its own disappearance is the R5 surprise the plan lists.
- "Ready to watch … until opened or dismissed": neither "opened" (the popup? the file?) nor
  "dismissed" (which key?) is defined, and nothing clears the state on the TUI side.

**Fix:** an open popup pauses the countdown (it is a glance, so it never *keeps* the daemon alive
for long, but it is also never cut short), the icon never hides while its popup is open, a short
"stopped" grace lets the owner see why the icon went, and the Ready icon state has an explicit
acknowledgement.

```diff
@@ -805,1 +805,2 @@
-| `interactive`: a TUI, or a visible dashboard window | busy | the daemon exits when the count drops to 0 | The bar never counts. |
+| `interactive`: a TUI, or a visible dashboard window | busy | the daemon exits when the count drops to 0 | The bar icon never counts. |
+| `glance`: the bar popup is open (`app.client {glance: true}`) | the countdown pauses while a glance is open and resumes where it was when it closes; a glance never starts the daemon and never counts as interactive | same | A popup that announces "stops in 4 min" must not stop while it is being read. |
```

```diff
@@ -1616,3 +1616,5 @@
 8. `hello` uses `kind: "bar"` and `interactive: false`. When the dashboard window becomes visible
    the service sends `app.client {interactive: true}`, and `false` when it hides, so the bar
-   never keeps the daemon alive on its own (A1).
+   never keeps the daemon alive on its own (A1). When the popup opens the service sends
+   `app.client {glance: true}` and `false` on close, which pauses the idle countdown (§3.2.10)
+   without counting as interactive.
```

```diff
@@ -1670,6 +1670,11 @@
-Visibility: `visible: svc.online || showWhenStopped`, default `false`, which keeps the owner's
-"visible only while the app runs" rule. Under A1 the icon therefore disappears a few minutes after
-the last download, and the menu, a keybinding or `fbittorrent dashboard` bring the app back. The
+Visibility: `visible: svc.online || showWhenStopped || root.opened || svc.stoppedRecently`,
+with `showWhenStopped` default `false`, which keeps the owner's "visible only while the app runs"
+rule. `root.opened` keeps the anchor of an open popup on the bar; `svc.stoppedRecently` is true
+for 10 s after a `bye`, during which the icon is dim with the tooltip "fbittorrent stopped
+(idle)" and the popup, if open, shows "fbittorrent stopped. Enter Start · w Open window". Under
+A1 the icon therefore disappears a few minutes after the last download, visibly rather than
+mid-glance, and the menu, a keybinding or `fbittorrent dashboard` bring the app back. The
```

```diff
@@ -1659,1 +1659,1 @@
-| Ready to watch (any row, until opened or dismissed) | play-circle-outline | `barForeground` | "Ready to watch: <name>" | as downloading |
+| Ready to watch (a torrent became playable and has not been acknowledged: acknowledged means the popup or the dashboard was opened after the event, `o` was pressed on it in any UI, or it completed) | play-circle-outline | `barForeground` | "Ready to watch: <name>" | as downloading |
```

P1-12 gains the case `daemon.py::idle_exit_glance_pauses` (open glance → countdown frozen →
close → resumes with the remaining time) and P2-04 gains `qs-visual.sh popup stopped-recently`.

### A7. ftui 0.6 `Table` sorts cell text lexicographically, so "sortable columns" is not what the plan needs

**Location:** §1.7 (347-352), §5.8 row "Torrent, search, release and feed lists" (2279), P4-04.

**Evidence:** `ftui-widgets-0.6.0/src/table.rs:228-259`: when `TableState.sort_column` is set, the
widget sorts by the cell's plain text with `Cow<str>::cmp`. "900 MiB" sorts after "1.1 GiB",
"12m" after "1h05", "—" anywhere. The widget's sort is usable for NAME only. `TableState` does
provide `handle_mouse` for header hits and `offset`/`selected`, which the plan can keep.

**Fix:**

```diff
@@ -2279,1 +2279,1 @@
-| Torrent, search, release and feed lists | `ftui_widgets::table` (sortable columns) over a `virtualized` list for long search results | P4-04 |
+| Torrent, search, release and feed lists | `ftui_widgets::table` for rendering, header hit-testing (`TableState::handle_mouse`) and the selection/offset state; **sorting is done by the TUI's store on typed values before building rows** (`sort_column: None`), because the widget's own sort compares cell text (`table.rs:228-259`) and would order "900 MiB" after "1.1 GiB". Only the visible slice of a long result list is turned into `Row`s; `virtualized` supplies the scroll arithmetic | P4-04 |
```

### A8. The Space exception is not forced by the platform

**Location:** §4.12 (1887-1889), Appendix B rows "Torrent: primary action" (4292), "Pause /
resume" (4294), exceptions paragraph (4325-4330).

**Evidence:** `Ui/PanelKeyCatcher.qml:289-295`: Enter emits `returnRequested()` and then
`activateRequested()`; Space emits `activateRequested()` only. A panel that sets a flag in
`onReturnRequested` and reads it in `onActivateRequested` tells the two apart; `returnRequested`
exists for exactly that. So the dashboard and the popup can keep Enter as the primary action and
make Space pause/resume, which is what the TUI does and what "same letters everywhere"
(principle 7) asks for. Today's popup already tries to map Space to pause (`Panel.qml:180`,
`t === " "`) and silently fails because the catcher consumes Space first; the plan would carry
that bug forward as a documented exception.

**Fix:**

```diff
@@ -1887,3 +1887,5 @@
-- Two platform exceptions to "same letters everywhere": in QML lists Space activates (Omarchy's
-  convention) where the TUI uses Space to pause; and the popup's Enter opens the file (as today,
-  `Panel.qml:72-76`) where the dashboard's Enter opens details. Both are listed in Appendix B.
+- Space pauses and resumes in every surface. `PanelKeyCatcher` emits `returnRequested` before
+  `activateRequested` for Enter and only `activateRequested` for Space
+  (`Ui/PanelKeyCatcher.qml:289-295`); the popup and the dashboard set a flag in
+  `onReturnRequested` and treat an `activateRequested` without it as Space. One exception remains
+  to "same letters everywhere": the popup's Enter opens the file (as today, `Panel.qml:72-76`)
+  where the dashboard's Enter opens details. It is listed in Appendix B.
```

```diff
@@ -4292,1 +4292,1 @@
-| Torrent: primary action | Enter opens details (Overview first) | Enter opens details | Enter or Space opens the file (exception 2) |
+| Torrent: primary action | Enter opens details (Overview first); double-click too | Enter opens details; double-click too | Enter opens the file (the one exception) |
@@ -4294,1 +4294,1 @@
-| Pause / resume | `p`, Space | `p` | `p` |
+| Pause / resume | `p`, Space | `p`, Space | `p`, Space |
```

```diff
@@ -4325,6 +4325,4 @@
-Exceptions to "same letters everywhere", both forced by the platform or by existing muscle memory:
-
-1. In QML lists Space activates (Omarchy's `PanelKeyCatcher` maps Enter and Space to activate,
-   `Ui/PanelKeyCatcher.qml:86-92`); in the TUI Space pauses or toggles.
-2. The popup's Enter opens the file (today's behaviour, `Panel.qml:72-76`); the dashboard's and the
-   TUI's Enter open details.
+One exception to "same letters everywhere", kept for muscle memory: the popup's Enter opens the
+file (today's behaviour, `Panel.qml:72-76`); the dashboard's and the TUI's Enter open details.
+(Space is not an exception: `PanelKeyCatcher` lets a panel tell Enter from Space through
+`returnRequested`, `Ui/PanelKeyCatcher.qml:289-295`, so Space pauses everywhere.)
```

### A9. Three key overloads the plan admits and defers

**Location:** Appendix B rows 4295, 4298, 4304, 4306, 4314, paragraph 4332-4335; §5.7 row 12.

**Evidence:**

- `u` is "seed on/off" on Torrents and "undo last remove while the undo toast shows" on the same
  screen (4298). A toast is up for 10 s after any removal; a `u` meant for the next row's seed
  switch undoes the removal instead, silently re-adding a torrent. Heuristic 5 (error prevention)
  on a key the plan itself made contextual.
- `Q` is "quit fbittorrent" everywhere and "quality" on the Shows list (4306), with the fallback
  "if the owner finds it confusing, quality moves to `=`" (4335). A plan should decide; the
  owner's muscle memory for `Q` is quit (`src/tui/mod.rs:1207-1212`).
- `s` is "sort" on Torrents and "providers" on Search (4304, 4314). Search results are the list
  the owner most wants to re-sort (seeds, size, date; omaqbt and ucmz851 both sort results).

**Fix:**

```diff
@@ -4295,1 +4295,1 @@
-| Seed on/off | `u` | `u` | `u` |
+| Seed on/off | `u` | `u` | `u` |
@@ -4298,1 +4298,1 @@
-| Undo last remove (new, P6-08) | `u` while the undo toast shows | the toast's button, `u` | n/a |
+| Undo last remove (new, P6-08) | `U` while the undo toast shows (never `u`, which is seed on the same screen) | the toast's button, `U` | n/a |
@@ -4304,1 +4304,1 @@
-| Sort (new) | `s` after P4-09 (today `s` is a legacy-only toggle); `S` reverses | `s`, `S` reverses | n/a |
+| Sort (new) | `s` on every list after P4-09 (today `s` is a legacy-only toggle): Torrents by Added, Name, Progress, ↓, Size, State, ETA; Search by Seeds, Size, Uploaded, Name, ★; `S` reverses | `s`, `S` reverses, header click | n/a |
@@ -4306,1 +4306,1 @@
-| Show quality (new, P3-18) | `Q` on Shows (a prompt) | the dropdown on the show row | n/a |
+| Show quality (new, P3-18) | `=` on a show row (a prompt); `Q` stays quit everywhere | the dropdown on the show row | n/a |
@@ -4314,1 +4314,1 @@
-| Search: browse lists / providers / source page | `b` / `s` / `O` | `b` / `s` / `O` | n/a |
+| Search: browse lists / providers / source page | `b` / `P` / `O` (`P` has no force-start meaning on Search) | `b` / `P` / `O` | n/a |
```

```diff
@@ -4332,4 +4332,3 @@
-`u` means "seed" on Torrents and "undo" in the Shows inbox and on an undo toast; all three are
-contextual and documented in Help. `Q` means "quit" globally and "quality" only on the Shows
-screen's show list, where the quit verb is on Ctrl+Q and in Help; §10.3 Q6's chord decision does
-not affect this. (If the owner finds the `Q` overload confusing, quality moves to `=`.)
+`u` means "seed" on Torrents and "undo the last inbox read" in the Shows inbox, where there is
+nothing to seed; `U` undoes a removal while its toast shows, on every surface. `Q` means "quit"
+everywhere; show quality is `=`, so the quit verb never changes meaning by screen.
```

§5.7 row 12 gains "`s` sorts every list; `U` is undo-remove" and P4-02's invariant test
`no_duplicate_key_per_scope` must treat "while the toast shows" as the same scope as the screen.

### A10. The 80-column mockups overflow 80 cells in four places

**Location:** §5.6 details modal (line 2242), Appendix C "TUI Search at 80×24" (4494, 4496),
"TUI Shows at 80×24" (4505, 4507).

**Evidence (display widths measured with East Asian Width rules, the same as `display_width()`):**
details-modal footer 82; Search row `[philosophy-raws]…` 81 and footer 83; Shows header 81 and
the South Park row 82. The brief for these mockups is "cell-for-cell"; a golden made from them
would wrap.

**Fix:**

```diff
@@ -2242,1 +2242,1 @@
-[Details] (O)pen  (P)ause  (U) Seed  (D) Remove  (i) IMDb  (w) Wiki     (Esc) Back
+[Details] (O)pen  (P)ause  (U) Seed  (D) Remove  (i) IMDb  …           (Esc) Back
@@ -4494,1 +4494,1 @@
-│   [philosophy-raws][辛特尔][sintel][BDRIP]…      963.2 MiB     1   —          │
+│   [philosophy-raws][辛特尔][sintel][BDRI…      963.2 MiB     1   —          │
@@ -4496,1 +4496,1 @@
-[Results] (Enter) Download  (v) Card  (F)ollow  (/) Query  (s) Providers … (?) Help
+[Results] (Enter) Download  (v) Card  (F)ollow  (/) Query  (s) Sort  …   (?) Help
@@ -4505,1 +4505,1 @@
-│      RELEASE SHOW                                   QUAL      SIZE   SEEDS    │
+│      RELEASE SHOW                                  QUAL      SIZE   SEEDS    │
@@ -4507,1 +4507,1 @@
-│   ●  S27     South Park (1997) · S27 1080p x265…  1080p   1.4 GiB    245    │
+│   ●  S27     South Park (1997) · S27 1080p x26…  1080p   1.4 GiB    245    │
```

(The Search footer also shows why A9 matters: with `s` = sort the footer reads the same on every
list.)

### A11. The 80×24 details modal shows a poster that cannot exist

**Location:** §5.6 details-modal mockup (2238-2240) versus §5.5.5 (2161-2162).

**Evidence:** §5.5.5 fixes posters at 24×18 cells (`src/tui/poster.rs:12-14`, a 2:3 image at
24×36 half-block pixels). The mockup draws a 12-cell-wide, 3-row poster: 12×6 pixels, which is
not an image. The kitty path cannot shrink below the cell footprint either (`place` uses
`c=area.width, r=area.height` on the 360×540 PNG, `poster.rs:88`).

**Fix:** no poster in the 80-column modal; the card is text, and `v` (P4-11) shows the full card
overlay when the owner wants the image.

```diff
@@ -2238,3 +2238,3 @@
-│ ▀▀▀▀▀▀▀▀▀▀▀▀ The Diplomat (2023)  TV series                                  │
-│ ▀▀▀▀▀▀▀▀▀▀▀▀ ★ 8.0 · 102k votes                                              │
-│ ▀▀▀▀▀▀▀▀▀▀▀▀ Keri Russell, Rufus Sewell                                      │
+│ imdb       The Diplomat (2023) · TV series · ★ 8.0 · 102k votes   (v) poster │
+│            Keri Russell, Rufus Sewell                                        │
+│            Political thriller: a veteran US diplomat is appointed ambassa…   │
```

### A12. `ConfirmDialog` does not know `y`

**Location:** §4.9 (1835-1836), Appendix B row "Confirm a destructive prompt" (4322).

**Evidence:** the shell's dialog handles Esc, Left/Right/Tab and Enter only
(`Ui/ConfirmDialog.qml:24-40`); a `y` reaches the dialog and does nothing. The plan's "`y`
confirms" needs the dashboard to intercept the key before `ConfirmDialog.handleKey`. Also
`onEntered: root.selectedIndex = index` (`:51`) moves the selection to whichever button the
pointer happens to rest on when the dialog opens, so "focus starts on Cancel" holds only until
the mouse moves; the P3-09 test "Enter right after opening cancels" must run with the pointer
parked outside the dialog.

**Fix:**

```diff
@@ -1835,2 +1835,5 @@
-- `d`, `x` and Delete open "Remove (keep files)"; `D` opens "Remove and delete files". Enter
-  activates the focused button (Cancel by default); `y` confirms.
+- `d`, `x` and Delete open "Remove (keep files)"; `D` opens "Remove and delete files". Enter
+  activates the focused button (Cancel by default); `y` confirms. The shell's dialog handles only
+  Esc, arrows, Tab and Enter (`Ui/ConfirmDialog.qml:24-40`), so the dashboard's key handler maps
+  `y` to `confirmed()` before delegating to `handleKey`. Hovering a button selects it (`:51`), so
+  the fixture test moves the pointer off the dialog before pressing Enter.
```

### A13. "∞" is listed as a marker the typography rules forbid

**Location:** §4.2.1 row "Unknown marker" (1408) versus §4.2.3 (1465-1466, "never '∞': absent ETA
renders '—'"). Fixed by the A2 hunk (the row now says "never ∞").

### A14. Appendix C's "idle" popup shows a countdown while a download runs

**Location:** Appendix C, first popup mockup (4350, 4357).

**Evidence:** the hero says "1 DOWNLOADING" and a row is downloading at 18 %, yet the dim line
reads "Idle · stops in 4 min after the download". Under §3.2.10 `downloading` is a busy reason, so
no countdown exists; the mockup contradicts the state machine it illustrates.

**Fix:**

```diff
@@ -4357,1 +4357,1 @@
-│ Idle · stops in 4 min after the download           │
+│ Ready to watch · press o to start the player       │
```

### A15. Acceptance lines that cannot be checked from a capture

**Location:** §0.2 "Look" row (78), P3-09 acceptance (3330-3331), P3-16 acceptance (3401).

**Evidence:** "indistinguishable from first-party panels", "leave the bar responsive (manual
check …)", "README reviewed" are judgements, not observations. P3-17 already has the right
mechanism for frames (`QSG_RENDER_TIMING=1`); the same can serve P3-09. First-party geometry can
be measured against the Wi-Fi panel in the same theme.

**Fix:**

```diff
@@ -78,1 +78,1 @@
-| Look | A careful TUI prototype [RC §9]; a popup built from shell components but fed by polling | Popup and dashboard indistinguishable from first-party panels (tokens, spacing, motion rules, states); a TUI that is legible at 80 columns on a monochrome theme | §4, §5 |
+| Look | A careful TUI prototype [RC §9]; a popup built from shell components but fed by polling | Popup and dashboard built only from shell tokens and components (lint), with the popup's hero title baseline, meta baseline, first separator and row pitch within 1 px of the Wi-Fi panel's in the same theme (`qs-visual.sh --compare omarchy.network`), every §4.11 state captured in the owner's theme and one light theme; a TUI whose state column reads at 80 columns on a monochrome theme (goldens) | §4, §5, P2-08 |
```

```diff
@@ -3330,2 +3330,4 @@
-- *Accept:* with 200 fixture torrents, scrolling and a burst of deltas leave the bar responsive
-  (manual check plus the "no JavaScript loop per delta" lint); every §4.11 state; Appendix B keys;
+- *Accept:* with 200 fixture torrents, 10 s of 1 Hz deltas plus continuous wheel scrolling
+  produce no frame over 32 ms in the test instance's `QSG_RENDER_TIMING=1` log (the mechanism of
+  P3-17) and the "no JavaScript loop per delta" lint passes; every §4.11 state; Appendix B keys;
```

```diff
@@ -3401,1 +3401,2 @@
-- *Accept:* smoke green; captures stored; README reviewed.
+- *Accept:* smoke green; captures stored; the README's dashboard section names every §4.11 state
+  and every Appendix B dashboard key (`agent_contract.py::readme_dashboard_keys` greps it).
```

And in §4.16 "Visual captures": add "`--compare <plugin>` captures the named first-party panel in
the same theme and reports the y of its hero title baseline, meta baseline, first
`PanelSeparator` and the pitch of its first two rows next to ours; a difference above 1 px is a
failure for the popup (P2-08)".

### A16. The bar's idle glyph is the upload arrow

**Location:** §4.5 icon table rows "Daemon not running" and "Running, nothing active" (1656-1657),
§4.2.4 row "idle / seeding" (1479).

**Evidence:** `󰕒` is `nf-md-upload`. In `screens/30-bar-right-widgets.png` the torrents widget is
an up-arrow between a cloud and a shield; it reads as "something is uploading", which is the one
thing seeding-off-by-default guarantees is not happening. The plan's own brand rule (§3.8 "the
glyph is the same shape as the bar's download glyph so the notification and the bar read as one
app"; Sprout's "one mark, state by weight") says the mark should not change shape for idle.

**Fix:**

```diff
@@ -1656,2 +1656,3 @@
-| Daemon not running | hidden by default; with `showWhenStopped`, `󰕒` | dim (`Qt.darker(barForeground, 1.4)`) | "fbittorrent is not running · click to start" | none |
-| Running, nothing active | `󰕒` | dim | "Idle · 8 done", or "Idle · stops in 4 min" | none |
+| Daemon not running | hidden by default; with `showWhenStopped`, `󰇚` | dim (`Qt.darker(barForeground, 1.4)`) | "fbittorrent is not running · click to start" | none |
+| Running, nothing active | `󰇚` (the brand mark; state is carried by weight, not shape) | dim | "Idle · 8 done", or "Idle · stops in 4 min" | none |
+| Seeding only (no download active) | `󰕒` | `barForeground` | "↑ 120 KB/s · 1 seeding" | "120K" |
```

---

## Section B. Improvements

Severity: P0 blocks the look-and-feel goal or a milestone demo; P1 the owner would notice within
a week; P2 polish with a measurable upside.

### B1 (P0). Prove the dashboard geometry, and make the chip-row tier the designed default for tiled use

Analysis: A1 is the defect; the improvement is to treat the 900-1099 tier as the tier the owner
will actually live in (the tiled window is about 960 px, `screens/tui-real.png`) and give it the
same care as the wide one: chips with counts, details as a sheet opened by Enter, NAME never
below 210 px, the key hint row intact. Rationale: a dashboard that is only good floating at 1600
px is a second TUI. Diff: the A1 hunks, plus the Appendix C "Dashboard at 900 px" mockup should
show ↑ and PEERS (they fit, 272 px for NAME remains).

### B2 (P0). One contrast ladder for QML and TUI, tested from one theme fixture

Analysis: A2 and A3. Rationale: the plan already wants numbers and state words shared between
`Model.js` and `format.rs` with one fixture; colour is the third thing that drifts, and it is the
one the owner's theme makes visible. Diff: the A2 and A3 hunks. One addition to P2-08's Tests:

```diff
@@ -3223,3 +3223,4 @@
-- *Tests:* `qs-format.sh` (formatting fixtures: sizes, rates, ETA, percent edge cases 99.5 and
-  0.04 %); `contrast.py --self-test` with a known-bad theme; a `qs-visual.sh --dry-run` that
-  checks every tool it needs (`grim`, `omarchy-shell`, `hyprctl`) and logs their versions.
+- *Tests:* `qs-format.sh` (formatting fixtures: sizes, rates, ETA, percent edge cases 99.5 and
+  0.04 %; `dim_ladder` and `accent_fallback` over `contracts/ui/themes/*.toml`, the same files
+  `palette::tests` reads); `contrast.py --self-test` with a known-bad theme; a `qs-visual.sh
+  --dry-run` that checks every tool it needs (`grim`, `omarchy-shell`, `hyprctl`) and logs their
+  versions.
```

### B3 (P1). "Ready to watch" must also say whether playback will outrun the download

Analysis: DL-26 fires at `max(32 MiB, 3 %)` plus the last piece. For a 1080p file at about
1 MB/s of video, 32 MiB is about 30 s of playback. If the link delivers less than the bitrate,
the player stalls within a minute, and the notification taught the owner nothing. The daemon has
everything needed for a better statement: the contiguous prefix, the file size, the current rate
and, when the container header gives a duration, the bitrate. The condition "the rest of the file
arrives before the player reaches the end of the prefix" is `prefix / bitrate ≥ (size − prefix) /
dl_bps`. Without a duration, `dl_bps ≥ size / assumed 2 h` is a crude fallback and should be
labelled as such. Rationale: this is the product's headline feature (A6, §2.7) and R15 already
admits the threshold is a guess; a second boolean turns a guess into an honest statement the
owner can act on, and keeps the threshold for the notification so nothing fires later than
today's plan. Fragments and qBittorrent have nothing like it; this is where "better than
qBittorrent" is earned.

```diff
@@ -4198,1 +4198,1 @@
-| `Playable` | `file: u64`, `ready: bool`, `prefix_bytes: bytes`, `buffered_s: u64?` |
+| `Playable` | `file: u64`, `ready: bool` (DL-26 threshold), `stall_free: bool?` (at the current `dl_bps`, the rest of the file arrives before playback reaches the end of the prefix: `prefix_bytes / bitrate ≥ (file_size − prefix_bytes) / dl_bps`; absent when no duration is known or the torrent is not downloading), `prefix_bytes: bytes`, `buffered_s: u64?`, `finishes_in_s: u64?` |
```

```diff
@@ -1197,3 +1197,6 @@
 Watch-readiness (P5-05) adds `playable` to the model: `{file: index, buffered_s?, ready: bool}`
 for the largest video file (by extension list `mkv mp4 webm avi mov m4v ts`), present only when
-the `watch` cap exists and metadata is known.
+the `watch` cap exists and metadata is known. `ready` is the DL-26 threshold and fires the
+notification once; `stall_free` is recomputed every second while downloading and is what the
+rows show: "Ready to watch · 4 min buffered" when `stall_free` is true or unknown with
+`buffered_s ≥ 120`, "Playable · may stall (finishes in 18 min)" otherwise. `o` works in both.
```

P5-05's Do gains: "`stall_free` and `finishes_in_s` from `dl_bps`, recomputed per second; the
popup, table and TUI row text use the two wordings above; `wait --until playable` keeps the
threshold semantics, `--until stall-free` is added". Tests: `daemon.py::playable_stall_free_flips`
(throttle the seed below the bitrate: `ready` true, `stall_free` false; raise the throttle:
`stall_free` true). §10.3 Q10 becomes "threshold for the notification, and should the row say
'may stall'?".

### B4 (P1). Disk-full behaviour, which the plan never mentions

Analysis: `Summary.free_space` exists and the add sheet shows free space (P6-11), but nothing
says what happens when `~/Downloads` fills: the engine will hit `ENOSPC`, the torrent enters
`error` with a raw message, and the owner finds out when the player stops. qBittorrent, Fragments
and Transmission all warn; for a streaming user with a 3.4 GiB library it is the likeliest
failure after the VPN. Rationale: principle 4 (truthful state) and heuristic 5 (error prevention)
at the one moment they pay off. Cost: one supervisor check every 5 s, one `reason`, one banner
in three surfaces, one add-sheet rule.

```diff
@@ -2364,1 +2364,2 @@
 | 36 (new) | Sleep inhibitor while downloading | none | In | T2 | A laptop must not suspend mid-download [FO §5]. | P6-09 |
+| 37 (new) | Low disk space warning and `disk_full` state | none; `ENOSPC` becomes a raw engine error | In | T1 | The likeliest failure after the VPN for a streaming user; qBittorrent, Fragments and Transmission all warn before it happens. | P5-07 |
```

New item after P5-06:

```diff
@@ -3619,1 +3619,17 @@
 - *Depends on:* P0-05, P1-02.
+
+**P5-07 Low disk space warning and `disk_full`** (S)
+- *Do:* The supervisor computes `needed = Σ (size − completed_bytes)` over running, incomplete
+  torrents and `statvfs` of `save_path` every 5 s. When `free_space < needed + 512 MiB`, `summary.disk`
+  = `{free_bytes, needed_bytes, low: true}` and every UI shows a banner "Not enough space for the
+  running downloads: 3.1 GB needed, 1.2 GB free" (urgent tint, §4.2.3); when the engine reports
+  `ENOSPC` the torrent's `state` is `error` with `reason: "disk_full"` and the word "Disk full".
+  The add sheet refuses with "Needs 8.9 GB · 1.2 GB free" in urgent when `size > free_space`;
+  `fbittorrent add` prints the same as a warning in JSON (`warnings[]`) and exits 0 (the engine
+  may still finish if the owner frees space). A notification "Disk almost full" fires once per
+  daemon life.
+- *Why:* §6.2 row 37; principle 4.
+- *Accept:* with a 64 MiB tmpfs as `save_path` and a 100 MiB fixture, the banner appears before
+  the first `ENOSPC`, the torrent ends in `error`/`disk_full`, and freeing space plus `start`
+  resumes it.
+- *Tests:* `daemon.py::{disk_low_banner, disk_full_reason, disk_full_recovers}` (tmpfs via
+  `unshare -m` or a loop file; skipped with a logged reason when neither is available); TUI and
+  popup goldens with the banner.
+- *Depends on:* P1-08, P2-03, P3-09.
```

Add `P5-07` to M4's dependency list, to §9.4's table (`P5-07 | P1-08, P2-03, P3-09 | M4`), to the
Daemon lane, and `disk_full` to the open `reason` enum in A.9 and the `Summary` object
(`disk: {free_bytes?, needed_bytes, low: bool}`). The P6-11 picker row keeps "free space" for
the destination, which is a different question.

### B5 (P1). The popup's quick add should preview before it commits

Analysis: §4.5 "`a` and the right click: quick add, immediate, as today". The clipboard can hold
anything; `a` is next to `s` and `d` on the home row; the magnet's `dn=` name is known before any
network traffic; and P6-13 already specifies a one-line offer strip "Add '<name>'? [Add]
[Dismiss]". Using that same strip for `a` costs one Enter, shows the owner what is about to be
downloaded ("deliberate downloads", principle 2), shows "Already in your list" in place, and gives
the two features one component. The handler and the CLI stay immediate: there the click or the
command is the deliberate act. Rationale: heuristic 5 and consistency with P6-13; omaqbt's magnet
confirm row (`omaqbt Panel.qml:86-99`) is the same idea and users liked it.

```diff
@@ -1800,1 +1800,3 @@
-- the popup's `a` and the right click: quick add, immediate, as today;
+- the popup's `a` and the right click: the offer strip of P6-13 with the magnet's `dn=` name (or
+  the `.torrent` file name), "Add '<name>'? [Add] [Cancel]" with Add focused; Enter adds, Esc
+  cancels; a duplicate shows "Already in your list: <name>" instead of the strip;
```

And in §4.5 "Rows show …" bullets, replace "add from clipboard" in the hero's `a` description with
"offer the clipboard's torrent (§4.8)". `widget_keys.sh::W-013` presses `a` with a magnet on the
fake clipboard and asserts nothing is added until Enter.

### B6 (P1). Space pauses everywhere; double-click is Enter

Analysis and diff: A8. Rationale: the one interaction model of [UX] applied to all three surfaces,
and a key the owner already uses in the TUI. Add to §4.3.2: "double-click on a row is Enter;
there are no context menus, as in every first-party panel".

### B7 (P1). Settle the key overloads and make the dashboard close like the TUI

Analysis and diff: A9. One more row: Appendix B "Close this UI" gives the dashboard "Esc at the
top level, or close the window". ucmz851's layered Esc is right inside the window, but a
long-lived window that closes on one Esc too many is a trap (heuristic 3); the TUI closes on `q`.
Rationale: same letters everywhere; a window the owner keeps open on a workspace should not
vanish on a reflex key.

```diff
@@ -4290,1 +4290,1 @@
-| Close this UI | `q`, Ctrl+C (no longer stops downloads) | Esc at the top level, or close the window | Esc |
+| Close this UI | `q`, Ctrl+C (no longer stops downloads) | `q`, or close the window; Esc at the top level clears the filter and selection and then does nothing | Esc |
```

Also new: `y` copies the selected torrent's or result's magnet to the clipboard (`wl-copy`) in the
dashboard and the TUI, as omaqbt (`y` on detail) and ucmz851 (one-click copy) do; the owner's agents
and browser flows want a magnet more often than a page URL. Appendix B row "Copy magnet (new) |
`y` | `y` | n/a".

### B8 (P1). One bar mark, state by weight

Analysis and diff: A16. Rationale: Sprout's "95 % Omarchy / 5 % brand" and the plan's own
notification-icon rule. The tooltip and the Ready/VPN/error glyphs keep carrying the words.

### B9 (P1). Glance pauses the countdown; the icon never hides under an open popup

Analysis and diff: A6. Rationale: R5 says the new lifetime "surprises the owner"; the surprise
the plan can prevent is the one it creates.

### B10 (P2). Search results that are already in the list say so

Analysis: the Shows inbox marks `added_before` with `+`; search results do not, although the
daemon holds the ledger and dedupes by btih (§3.7). A row marked `+ in your list` prevents the
second most common mistake after a wrong click (adding the same release from another provider)
and answers "did I already grab this?" without switching screens. Cost: one boolean per result.

```diff
@@ -4210,1 +4210,1 @@
-| `SearchResult` | `key: str`, `name: str`, `size: bytes?`, `seeds: u64?`, `leechers: u64?`, `uploaded_at: ts?`, `providers: [str]`, `magnet: str?`, `url: str?`, `page_url: str?`, `imdb_id: str?`, `rating: f64?` |
+| `SearchResult` | `key: str`, `name: str`, `size: bytes?`, `seeds: u64?`, `leechers: u64?`, `uploaded_at: ts?`, `providers: [str]`, `magnet: str?`, `url: str?`, `page_url: str?`, `imdb_id: str?`, `rating: f64?`, `added: hash?` (the ledger entry with the same btih, when one exists; the row shows `+` and Enter opens that torrent instead of adding) |
```

P3-12's acceptance gains "a result whose btih is in the ledger shows `+` and Enter selects the
existing torrent".

### B11 (P2). Notification hygiene for agents, and a default action

Analysis: §3.8 notifies "Added" for every CLI add; an agent adding ten episodes produces ten
notifications with Undo buttons. `add --quiet` exists. Also `notify-send` invokes the action named
`default` on a click of the notification body; "Download complete" and "Ready to watch" should
open on click, not only through the button.

```diff
@@ -1257,1 +1257,1 @@
-  | added (handler and CLI) | "Added" | `undo` (remove, keep files), `window` (open dashboard) | P1-13 |
+  | added (handler, and the CLI when stdout is a TTY or `--notify` is given; agents and `--quiet` get none) | "Added" | `undo` (remove, keep files), `window` (open dashboard) | P1-13 |
@@ -1259,2 +1259,2 @@
-  | completed | "Download complete" | `open`, `folder` | P1-13 |
-  | ready to watch | "Ready to watch" | `open` | P5-05 |
+  | completed | "Download complete" | `default` and `open` (play), `folder` | P1-13 |
+  | ready to watch | "Ready to watch" | `default` and `open` (play) | P5-05 |
```

### B12 (P2). Key hints that look first-party, and the first-party popup width

Analysis: no first-party popup renders a key-hint row (grep over `network`, `bluetooth`,
`tailscale`, `agents`: none); they rely on `j/k/Enter/Esc` being the shell convention. The plan's
`KeyHint` row is right for discoverability (heuristic 6; the popup has eight letter keys), but as
drawn ("a Add / Search w Window t Terminal z Turtle" in `Style.font.body` with accent keys) it is
a toolbar, the "card-dashboard" look Sprout's rules reject. omaqbt's precedent is one row with a
single `w` hint. Also every first-party popup is `Style.space(380)` wide
(`panels/network/Panel.qml:1109`, `bluetooth:670`, `tailscale:421`, `agents:972`); ours keeps 400
from today.

```diff
@@ -1458,1 +1458,1 @@
-| Key hint ("p pause") | `Style.font.body` | key in accent, label in foreground | (omaqbt `TorrentTable.qml:324-338`) |
+| Key hint ("a add · / search · w window") | `Style.font.caption`, one line, rendered like `PanelSectionHeader` (bold caption, letterSpacing 1.2, no uppercase) | keys in foreground, labels dim; accent only when it passes §4.2.1 | omaqbt's table uses body-size hints (`TorrentTable.qml:324-338`); the popup keeps it to one caption line so it reads as section chrome, not a toolbar |
```

```diff
@@ -1432,2 +1432,3 @@
-- The popup is one column `Style.space(400)` wide (as today, `Panel.qml:162`) with
+- The popup is one column `Style.space(380)` wide (every first-party popup's width:
+  `panels/network/Panel.qml:1109`, `bluetooth:670`, `tailscale:421`; today's 400, `Panel.qml:162`, was arbitrary) with
```

### B13 (P2). The sidebar never shifts under the cursor

Analysis: §4.6 lists "Ready (P5-05, only when non-zero)" and "Waiting for VPN (only when
non-zero)". A row that appears and disappears moves every row below it under a `j/k` cursor and
under the mouse. Transmission's filter bar keeps all entries and greys the empty ones.

```diff
@@ -1766,3 +1766,3 @@
-- **Sidebar:** state filters with live counts: All; Downloading (downloading, stalled, metadata,
-  checking); Ready (P5-05, only when non-zero); Queued; Paused; Done; Seeding; Errors; Waiting for
-  VPN (only when non-zero). Later "From: Search, RSS, Shows, Manual" (P6-08). Then links to Shows
+- **Sidebar:** state filters with live counts, always in the same order so rows never move under
+  the cursor: All; Downloading (downloading, stalled, metadata, checking); Ready (P5-05); Queued;
+  Paused; Done; Seeding; Errors; Waiting for VPN (only while binding is configured). A filter with
+  a zero count is dim and still selectable. Later "From: Search, RSS, Shows, Manual" (P6-08). Then links to Shows
```

### B14 (P2). Make the remaining streaming questions explicit decisions

Analysis: a streaming-first client's feature table should say what it decided about subtitles,
quiet hours and the player, even when the answer is "no". Otherwise round 3 asks again.

```diff
@@ -2365,1 +2365,4 @@
+| 38 (new) | Subtitle fetch | none | Out | - | OpenSubtitles needs an account and an API key, which the owner excluded (§2.5); the player's own subtitle search covers it. | Revisit if a keyless source appears |
+| 39 (new) | Quiet hours for notifications | none | Out (nothing to build) | - | The shell's notification service owns Do Not Disturb (`plugins/bar/indicators/Dnd.qml`), and `notify-send` goes through it. | - |
+| 40 (new) | Player command | `xdg-open` | In | T2 | `open_command = "xdg-open"` in `Settings`; mpv users set `mpv --keep-open=yes` so a growing file does not end playback early. One config key, no UI beyond Settings. | P3-15 |
```

### B15 (P2). Accent fallback should test hue, not only luminance

Analysis: §5.5.1 switches keys to bold foreground when `accent` is "within 2.0:1 of
`foreground`". Measured: tokyo-night 1.19, catppuccin 1.46, gruvbox 1.37, kanagawa 1.00. On
tokyo-night the accent is blue on a grey-white foreground: the luminance ratio is low but the hue
difference is large and the key letters would stand out fine. The rule as written removes accent
from keys on most dark themes, which makes the TUI look less like the shell's own panels (which
use accent). The §5.5.2 channel-difference test is the better metric.

```diff
@@ -2106,1 +2106,1 @@
-| accent (focused border, keys, sorted header) | `accent` | if `accent` is within 2.0:1 of `foreground` (the owner's `#8d8d8d` on `#ffffff` text), keys use bold `foreground` instead of accent so they still stand out |
+| accent (focused border, keys, sorted header) | `accent` | if `accent` and `foreground` differ by less than 32 in every channel **and** are within 1.3:1 of each other (the owner's `#8d8d8d` against `#ffffff` qualifies by luminance; tokyo-night's blue accent does not), keys use bold `foreground` instead of accent; and whenever `accent` is below 4.5:1 against `background` (`catppuccin-latte`, `miasma`, `rose-pine`), keys and the ready badge use bold `foreground` |
```

### B16 (P2). Live "buffered" on every row, not only at the moment it became ready

Analysis: §4.3.1 shows "Ready to watch · 18 % · 4 min buffered" and §4.5 the `[Open]` badge, but
nothing says `buffered_s` keeps changing on the row. Since `playable` is a `Torrent` field and
deltas coalesce rate-like changes to the interval, it costs nothing to say so, and it is the one
number that tells the owner whether to press `o` now or in two minutes.

```diff
@@ -1518,3 +1518,4 @@
   · ETA · peers (and "seeds" after P5-01) · "Ready to watch" badge (P5-05). Absent fields are
-  omitted, never zero ("Fetching metadata · 0:12" shows elapsed time instead).
+  omitted, never zero ("Fetching metadata · 0:12" shows elapsed time instead). The badge's
+  "4 min buffered" is `playable.buffered_s` and updates with every delta (coalesced to the
+  interval like a rate), so the owner can watch the buffer grow before pressing `o`.
```

---

## Section C. Questions only the owner can answer

1. **Player.** Should `o` use `xdg-open` (whatever is registered for `.mkv`), or an explicit
   command such as `mpv --keep-open=yes`? mpv keeps playing a growing file; some players stop at
   the first EOF.
2. **Ready-to-watch wording.** Notify at the threshold (as planned) but label the row "may stall"
   when the current rate is below the bitrate (B3), or notify only when playback cannot stall?
3. **Popup quick add.** One key (`a`, immediate, as today) or `a` then Enter with the name shown
   (B5)? The clipboard watcher (P6-13), if you turn it on, removes the first key anyway.
4. **Space.** Space pauses in the dashboard and the popup as in the TUI (B6), or Space activates
   as in the Wi-Fi panel?
5. **Undo and quality keys.** `U` for undo-remove and `=` for show quality (A9), or other letters?
6. **Idle exit while glancing.** Pause the countdown while the popup is open and keep the icon 10 s
   after a stop (B9), or let Q2 (`showWhenStopped`) settle it by keeping the icon always?
7. **Peers tab tier.** The true seed count (P5-01) fixes D6; is the Peers tab itself T1 for you, or
   can P5-02 be T2 (dashboard first) so M4 ships earlier?
8. **Tiled or floating.** Will the dashboard mostly be tiled at about 960 px (then the chip-row
   tier is the one to perfect) or floated wide?
9. **"In your list" on search results** (B10): wanted, or is the duplicate answer on add enough?

---

## Appendix: what was checked and found right

So the integrator does not re-verify it: every `Style.*`, `Color.*`, `Border.*` line cited in
§4.2.1 exists at the cited line (`Style.qml:33,40-44,84-89,191-201,215-225,236-245,247-252,
257-262,271-277,329-340,363-367`; `Color.qml:19-23,84-108`; `Border.qml:17,141,217`);
`Ui/PanelHero.qml:23` and `Ui/PanelSectionHeader.qml:20` use `Qt.darker(foreground, 1.4)`;
`ConfirmDialog.qml:12` defaults `selectedIndex: 1` and `:52` caps the card at `Style.space(370)`;
`PanelKeyCatcher.qml:36-99` and `FilterInput.qml:3-6` are as described; `plugins/bar/Bar.qml:86-91`
exposes `foreground`, `barForeground`, `foregroundAnimationEnabled`, `urgent`;
`GalleryPanel.qml:286-289` and `303-320` match the dismiss and `Keys.AfterItem` claims;
`shell.qml:1161-1172` and `1356-1372` and `PluginRegistry.qml:118-133,148-163` match the summon
routing claims; `PluginShellApi.qml:29-31` has `serviceFor`. The popup row formula "content +
`Style.spacing.rowPaddingX`" (§4.3.1) is the first-party convention (`panels/network/Panel.qml:1893`),
not a token misuse. omaqbt `TorrentTable.qml:9-15,36-38,48-56,65-67,69-72,120-135,166-179,
205-217,225-226,285-341`, `Client.qml:736,763,803`, `WmFocus.qml:8-17,85-130`, `Panel.qml:86-99`;
Sprout `Panel.qml:1027,1047-1056,1072-1104,1111-1114,1197-1250`, `BarWidget.qml:306-320`,
`Dashboard.qml` callouts at `0.10`/`0.35`; Fragments `torrent_row.rs:260-290`, `utils.rs:127-139`,
`connection_manager.rs:50-55`; Transmission `FilterBar.cc:474-482`, `Notify.cc:234-253`;
qBittorrent `proptabbar.cpp:50-100`: all as cited. ftui 0.6 ships every widget §5.8 names
(`table`, `tabs`, `sparkline`, `virtualized`, `log_viewer`, `toast`, `notification_queue`, `badge`,
`scrollbar`, `command_palette/`; `ftui-extras` `image`, `canvas`), and `Cargo.toml:25-26` enables
`canvas` and `image`. The TUI's 80-, 120- and 160-column layouts fit except the four lines in A10;
the dashboard-at-900 and popup mockups are consistent in width. D1 to D18 and UX defects 1 to 15
each map to an item with a named test; the only gaps are the ones new defects A6 and A9 introduce.
