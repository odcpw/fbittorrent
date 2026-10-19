# Native capture matrix

Run only in the private compositor and network/XDG namespaces. Never summon or
reload the owner's shell and never switch their theme. PNGs and pair images go
to `target/test-logs/visual/<date>/`; JSONL goes to `qs_visual_runs/`. No capture
belongs under docs or in git. The owner's judgment of a pair remains manual.

For each available live state, run both:

```sh
FBT_TEST_VISUAL=1 plugin/tests/qs-visual.sh popup downloading --light owner --compare omarchy.network --binary target/debug/fbittorrent
FBT_TEST_VISUAL=1 plugin/tests/qs-visual.sh popup downloading --light catppuccin-latte --compare omarchy.network --binary target/debug/fbittorrent
```

Use `empty` or `paused` in place of downloading. Those recipes construct real
engine records in `visual_fixture.running`; `daemon.py --serve-fixture <state>`
holds the same fixture until stdin EOF. `table-200` uses the actual recipe
`regression_tests/fixtures/states/table-200.json`. Other names below are the
required matrix, with their current qualification stated explicitly. A missing
recipe is not replaced by a simulated daemon or a decorative screenshot.

| Surface | State | Fixture recipe / qualification |
|---|---|---|
| popup | no torrents | `empty`: real daemon, no records |
| popup | downloading | `downloading`: real private tracker/seed, 32 KiB/s |
| popup | paused/collapsed | `paused`: real saved pause intent |
| popup | not running | close the owned daemon; Start/window hint; live key suite |
| popup | stopped recently | real `app.shutdown`, ten-second interval; live W-016 |
| popup | starting | presentation unit; live state capture pending |
| popup | connecting | presentation unit; live state capture pending |
| popup | reconnecting | presentation unit; live state capture pending |
| popup | protocol major mismatch | presentation unit; real incompatible-server recipe pending |
| popup | version skew | presentation unit; two-binary recipe pending |
| popup | parked for VPN | presentation unit; private interface recipe pending |
| popup | engine failed | presentation unit; real failed-engine recipe pending |
| popup | mutation result unknown | client timeout control; live capture pending |
| popup | upgrade pending | presentation unit; lifecycle service integration pending |
| popup | idle countdown | presentation unit; optional `summary.daemon.idle_exit_in_s`; live integration pending |
| popup | clipboard magnet | real wl-paste offer in W-013; opt-in clipboard service pending |
| popup | quick add offer | real wl-paste offer in W-013 |
| popup | playable may stall | actual PopupRow presentation control; watch service capture pending |
| popup | disk low | presentation path; disk service capture pending |
| popup | disk full | real disk refusal fixture pending |
| popup row | cursor | actual keyboard cursor W-001..003 |
| popup row | pending | actual confirmed client action path; held-response capture pending |
| popup row | error | frozen error row/presentation text; real restore-error capture pending |
| popup row | done | real completion recipe capture pending |
| popup row | ready to watch | actual PopupRow contrast control; watch service capture pending |
| dashboard | no torrents | native empty view; empty real daemon |
| dashboard | filter matches nothing | native filter against `paused`; dashboard table suite |
| dashboard | downloading / paused | `downloading` / `paused`; dashboard table suite |
| dashboard | not running / starting / connecting / reconnecting | native empty-state paths; full capture set pending |
| dashboard | protocol mismatch / version skew / parked / engine failed | service state paths; full real capture recipes pending |
| dashboard | result unknown / upgrade pending / idle countdown | service/client state paths; lifecycle capture integration pending |
| dashboard | clipboard / offer / playable / disk low / disk full | named service-view beads; real captures pending |
| icon | not running (showWhenStopped) | `qs-icons.sh` stopped profile, actual BarIndicator presentation |
| icon | idle | `qs-icons.sh` idle profile |
| icon | seeding only | `qs-icons.sh` seeding profile |
| icon | stopped recently | `qs-icons.sh` stopped-recently profile; live W-016 lifetime |
| icon | downloading | `qs-icons.sh` downloading profile; real downloading capture |
| icon | ready to watch | `qs-icons.sh` ready profile; real watch capability pending |
| icon | waiting for VPN | `qs-icons.sh` parked profile; real private-interface capture pending |
| icon | error | `qs-icons.sh` error profile; real failed-engine capture pending |
| icon | turtle on | `qs-icons.sh` turtle profile; real limits capability pending |

`qs-icons.sh` renders actual native icon components in owner/latte palettes from
frozen wire-shaped presentation objects; it does not qualify live engine states.
`contrast.py --installed --self-test` checks the fixture/installed TOML palettes,
including the low-chroma owner theme and a deliberately failing palette.
