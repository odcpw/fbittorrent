# Handoff — agent-ergonomics pass 1 (2026-09-03)

Applied on `main`, no branches, workspace in-tree. Binary: `~/.local/bin/fbittorrent`.

## What an agent should know

- Start with `fbittorrent robot-docs` (handbook) or `fbittorrent capabilities --json` (contract 1.0.0).
- Canonical download flow: `triage --json` → `search "<title>" --json` → `add "<magnet>" --wait --json` → `show <hash> --json` → `open <hash>`.
- Exit codes: 0 ok · 2 usage · 3 safety (needs `--yes`) · 4 backend down · 5 upstream · 6 not found · 7 ambiguous · 8 timeout.
- Errors in `--json` mode are JSON on stdout with the same exit code and a `hint` that is a runnable command.

## Applied (R-001 … R-012, see applied_changes.jsonl)

Mega-command `triage`, `capabilities --json`, `robot-docs`, non-TTY bare invocation,
exit-code dictionary + typed errors + JSON error envelope, `--json` on every command,
`add` returns the hash and can `--wait`, new `wait` and `show`, `delete --files` gated
by `--yes` with `--dry-run`, name-substring selectors with ambiguity reporting, aliases.

## Reality-check follow-up (same day)

Six gaps found by the reality check were fixed directly, no beads: secure WebUI
defaults in `setup` (CSRF + host-header validation on), `.torrent` file input for
`add` (multipart upload, `file://` accepted), a lock around the ratings-dataset
refresh, JSON-only stdout for `setup --json`, per-add F2/F3/F4 toggles in the TUI
prompt, and three test layers (`cargo test` with a mock WebUI, `tui_smoke.sh`,
`widget_keys.sh`; `all.sh` runs them all). Two widget nits surfaced by the keyboard
test were fixed too: the cursor parks on the rows once they arrive, and the header
index resets on open.

## Deferred to pass 2

- `search` in the CLI still runs synchronously up to `--wait` seconds; a `--stream` NDJSON mode would let agents act on early results.
- A `doctor` command (backend health, plugin health, dataset freshness) — `triage.warnings` covers the basics.
- Per-command JSON schema export (`capabilities --schema`) if a consumer needs strict typing.
- The regression suite needs a reachable backend for R-012; a fixture-only mode would make it CI-friendly.

## Ambition bar

12 substantive surface changes across 7 dimensions, each pinned by a regression test.
The "That's it??" self-prompt was run once; the second round produced `show`, `wait`
and the name selectors.
