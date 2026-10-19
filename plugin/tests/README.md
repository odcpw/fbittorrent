# Isolated shell checks

The tests copy the plugin into a disposable configuration with private HOME,
TMPDIR and all five XDG directories. They never reload the installed shell.
`FBT_PLUGIN_SOURCE` selects a development copy; the default is this worktree's
plugin. Quickshell, Python, Node and the installed read-only Omarchy components
are required.

- `qs-client.sh`, `qs-store.sh`: API session and frozen transcript reducer checks.
- `qs-glyphs.sh`, `qs-format.sh`: actual Qt text/format/contrast checks.
- `contrast.py --self-test`: checked-in theme oracle and known-bad control.
  `--installed` additionally checks current installed themes; `--refresh` explicitly
  copies them and the owner's current colors into `contracts/ui/themes/`.
- `service-tests.py <debug-binary>`: an owned real daemon and two widget instances.
- `FBT_TEST_VISUAL=1 qs-visual.sh popup downloading --binary <debug-binary>`:
  actual native popup over a real local seed/client transfer, inside a private
  headless Sway compositor using pixman. No owner display socket is inherited.
  `--light catppuccin-latte` selects the private theme copy; `--compare
  omarchy.network` adds the installed reference panel and geometry annotations.
  The reference has an unavailable private network backend, so it never scans or
  changes the owner's network. The comparison image is for human review, without
  an automated claim that the popup looks first-party.
- `qs-visual.sh --dry-run`: read-only tool diagnostics. `qs-icons.sh` / `qs-visual.sh
  bar downloading`: all bar presentation profiles in dark/light. Bar profiles
  derive from the frozen wire objects; they do not prove live engine transitions.

Native popup captures require `sway` on PATH or `FBT_TEST_SWAY=/absolute/path/sway`.
On this Arch machine, `python3 plugin/tests/prepare-headless.py --download` can
extract packages into ignored `plugin/tests/target/headless-tools/` without sudo
or a system install. It checks the SHA-256 values from the local pacman sync
metadata before extraction. Normal tests never download tools automatically.
A missing headless compositor fails; there is no fallback to the owner's display.

Images and previous-image pixel differences go to `target/test-logs/visual/<date>/`.
JSONL, daemon, compositor and QML logs go to `target/test-logs/<suite>/`.
`daemon.py <debug-binary> --serve-fixture empty|paused|downloading` exposes the same
real fixture as `FBTFIXTURE {runtime, root, state, row}` on stdout; close stdin to
stop and reap all owned children. It never fabricates states for a screenshot.
