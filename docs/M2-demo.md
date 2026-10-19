# M2 dashboard demo

Use the matching installed binary and plugin: `fbittorrent dashboard`. Choose a
legal test download and a show to follow. Upload defaults **off**; Add has no
upload switch. Explicit per-torrent opt-in is **Upload: off/on** in Details.

| Journey | Actions and expected result |
|---|---|
| **W1 — find** | **2**, type a title such as Sintel, **Enter**. Inspect size, sources, date, rating and poster. Results arrive progressively and duplicate sources share one result. **j/k** selects; **/** edits the query; **Ctrl+A** replaces it. |
| **W1 — review** | **Enter** opens Add; inspect destination/files. **Esc** discards. Reopen, then **Enter / Download** once. A clicked magnet follows the same route: its name appears immediately with “Fetching file list…” if needed. Download works before metadata, commits once and closes the sheet. |
| **W1 — watch** | **1**, select the download, **Enter** for Details. **[ / ]** selects Files; **j/k**, **Space** include/skip a file. Return to Overview. Progress updates within two seconds. When **Ready to watch** appears before completion, **o** opens the real video; check actual playback. |
| **W1 — browse/live settings** | In Search: **/**, **Ctrl+A**, **Backspace**, **Enter** opens browse posters. **,** opens Settings; change Active downloads and **Enter** applies it live. Return to Torrents and resize: below 1400 px Details becomes a sheet; below 1100 px sidebar controls become chips. **q** closes the window while transfer continues. |
| **W2 — inbox** | **3**, **n**, title/IMDb ID, **Enter** follows; **r** checks without downloading. **h/l** selects shows/releases; **j/k** moves. **=** sets preferred quality; **r** applies it on the next check. Read state / Release quality filter the inbox. Unread counts agree; newest releases appear first. |
| **W2 — read/review/persist** | **m** marks read; **u** undoes. **Enter**, **Esc** reviews/discards without downloading or marking read. Review again, then **Enter / Download** commits and marks read. Close/reopen; later deliberately restart the daemon. Follow, preference, read state and transfer association persist. A background release notification returns to Shows; an open inbox suppresses redundant notifications. |
| **W8 — remove/undo** | On a disposable watched torrent: **d**, **Enter** cancels unchanged. Repeat **d**, explicitly choose Remove: files remain. **U** restores within 10 s. Filter with **/**; **Ctrl+A** selects matching rows for Remove/Undo. Expired Undo cannot restore a record. |
| **W8 — delete** | Only on disposable files: **Shift+D**, **Enter** cancels. Reopen and explicitly confirm Delete. Only selected torrents’ owned files disappear; unrelated files and unselected torrents remain. |

Owner check: judge readability/focus and light/dark appearance at
900/1280/1500/1600 px, real provider results, notification return and video
playback. Automated journeys use private Sway/XDG, local services and recording
launchers; these human/public-network checks remain yours. Report the first
unexpected step rather than repeating a download or deletion.

Closing a window keeps downloads running. **Ctrl+Q** or the popup power button
stops the daemon and all transfers; the bar stops showing it as running.

Automated qualification:
`python3 plugin/tests/m2.py --binary /path/to/current/fbittorrent`.
Use `--group p2|dashboard|daemon` or repeated `--case NAME` for a subset.
`target/m2-*/summary.json` lists commands, results and evidence; the table case
stores `dashboard-{900,1280,1500,1600}.png` with Details open.
