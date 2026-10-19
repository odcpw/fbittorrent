# Owner checks

These are the human milestone checks; automated tests do not tick them off.
Record the date, `fbittorrent --version` and the first unexpected step. Use
disposable test downloads you choose, with **Upload: off**. Run shutdown checks
when stopping all transfers is intended. Restore themes, limits and preferences
after checking them. Stop at an unexpected add/delete result rather than retrying.

- [ ] **M1 — popup readable in your theme and catppuccin-latte.**
  1. In a terminal, save the theme name: `theme_before=$(omarchy-theme-current)`.
     Run `fbittorrent dashboard` and start a test download. Click the bar icon;
     inspect the rates, progress, full-name hover text and **j/k** selection.
     Pause/resume the test row; expand the Paused section when collapsed. Open
     another magnet for review, inspect Reviewing, then Discard. Inspect a Done
     row after completion and any visible VPN/error explanation. Check labels,
     dim text, focus, cancel/confirm controls and the power button for readability.
  2. Run `omarchy-theme-set catppuccin-latte`. Reopen the popup and repeat those
     checks, including hover/selection and confirmation with Cancel selected.
     Record any text or control that disappears into the light background.
  3. With only intended transfers to stop, click power and confirm if asked.
     Check that transfers stop and the icon becomes unlit/disappears. Reopen
     the offline popup if available: it must not show an idle-exit countdown.
  4. Restore the theme with `omarchy-theme-set "$theme_before"`. Mark this item
     only after both themes are readable; keep a screenshot of any failed state.

- [ ] **M2 — dashboard demo without a hitch.**
  1. Run [the M2 demo](M2-demo.md) in order: W1 search → review → Download →
     Ready to watch/Open; W2 follow → check → quality → read/undo → review and
     commit; W8 Cancel → Remove → Undo → explicit Delete of disposable files.
  2. Resize through 900, 1280, 1500 and 1600 logical pixels. Check readable names,
     sidebar chips below 1100, Details as a sheet below 1400 and docked above it.
     Close/reopen the window during an active transfer; progress must continue.
  3. Drag a local `.torrent` from the file manager onto the real dashboard.
     Check the review name/destination/files, then Esc/Discard. Check that no
     transfer was committed and existing files remain unchanged.
  4. Close/reopen Shows and deliberately restart the daemon when appropriate;
     confirm follow, quality, read state and transfer association persist. Mark
     this item only after the whole demo, actual player and desktop focus work.

- [ ] **M3 — TUI readable at 90 columns over SSH.**
  1. On another machine, resize the terminal to 90 columns; `stty size` must
     report `90` as its second number. Connect with
     `ssh -t DESKTOP_HOST 'fbittorrent tui'` (replace `DESKTOP_HOST`).
  2. Inspect an active test torrent and saved paused/done rows. Every row must
     show a readable state word. Move with **j/k**, open Details with **Enter**,
     cycle tabs with **[ / ]**, and return with **Esc**. Check that names,
     progress and contextual hints fit, without overlapping adjacent columns.
  3. Visit **1–4** and **?**; type/edit a search query. Check that pushed updates
     preserve your selection and input focus, and that the SSH terminal stays
     responsive. Close with lowercase **q**. If the test transfer is still
     incomplete, `ssh DESKTOP_HOST 'fbittorrent status --json'` must show the
     daemon still running; reopening the TUI must show continued progress.

- [ ] **M4 — watch before finish, then see “may stall” with turtle.**
  1. Record the current turtle state and download limit in Settings. Start a
     sufficiently large legal MKV/MP4 film with a known duration and average
     bitrate above 16 KiB/s. Keep Upload off and turtle off; allow enough source
     bandwidth for playback readiness before completion.
  2. While progress is below 100%, wait for **Ready to watch**. Press **o** and
     check actual playback. Record the percentage and a screenshot; a badge
     first appearing at completion does not satisfy this check.
  3. In Settings, set **Turtle download limit** to `16K`, return to the torrent
     and press **z** to enable turtle. Wait for the measured rate to settle
     below the film's bitrate. While still incomplete, the row must change to
     **Playable · may stall**, with the known buffer/finish estimate reflecting
     the slower transfer. Record the row; unknown duration/rate is not proof.
  4. Restore the prior turtle limit and state. Mark this item only after both
     readiness before completion and the slower-transfer warning were observed.

- [ ] **M5 — clipboard watch useful for a day.**
  1. Record the existing preference. Start with `fbittorrent up`; opt in with
     `fbittorrent settings --clipboard-watch true`, then open dashboard or TUI.
     Watching operates while the daemon runs; normal idle exit still applies.
  2. Copy ordinary text: no offer. Copy a new legal magnet: one correctly named
     offer, with no torrent added yet. Copy it again: no duplicate prompt.
     Dismiss with **Esc**; close/reopen the UI and check it stays dismissed.
  3. Copy a different test magnet and accept using the displayed Add control
     (**y** in the TUI). Check exactly one correct torrent appears with Upload
     off. Copy an already-added magnet: no redundant offer. While editing a
     query or using a dialog, check that offers do not steal text-entry keys.
  4. Use the app normally for a day. Note whether offers save effort, interrupt
     work or repeat unexpectedly. Check Log for raw clipboard text: none should
     appear. Disable watching with
     `fbittorrent settings --clipboard-watch false` and copy a fresh magnet:
     no new offer. Restore your preferred setting and record the usefulness
     verdict before marking this item.
