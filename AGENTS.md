# AGENTS.md — how agents work in this repository

fbittorrent is a Rust torrent app for the Omarchy desktop: one `fbittorrent` binary (CLI,
FrankenTUI terminal UI, embedded librqbit engine on the Asupersync runtime), a Quickshell bar
widget in `plugin/oliver.torrents/`, and vendored crates under `vendor/`. The implementation plan
is `docs/plan-2026-10-18/PLAN.md`; the work is tracked as beads in `.beads/` (`br`). Read
`README.md` once. Everything you need for a bead is inside the bead itself.

## Environment

```bash
export PATH="$HOME/.rch/shims:$HOME/.cargo/bin:$PATH"   # rch cargo shim first, then the rustup proxy
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"        # this box has 8 cores shared by several agents
```

- `cargo` goes through `rch` (remote compilation on the `3090` worker over Tailscale) and falls
  back to a local build when no worker is admissible. Do not fight it; do not set
  `RCH_ENABLED=0`. Toolchain is Rust 1.98.0 (rustup default; `rust-toolchain.toml` pins it once
  P0-05 lands).
- Prefer `cargo check` and `cargo test` (debug) while developing. The release build takes about
  7 minutes; run `cargo build --release --locked` only when a bead's acceptance needs the release
  binary (TUI smoke tests, `native.py`).
- Build cadence: do not build after every small edit. Finish a whole step or a coherent group of
  changes, then `cargo check` once; run `cargo test --locked` and the e2e scripts when a feature is
  complete. A handful of builds per bead, not dozens; the build path is shared.
- Memory: a release build of this crate peaks above 11 GB and systemd-oomd kills whole scopes when
  the desktop slice is under pressure (it killed the first swarm on 2026-10-18). Run every release
  build or `all.sh` as
  `flock -w 7200 /tmp/fbittorrent-release.lock systemd-run --user --scope -p MemoryHigh=10G -p MemoryMax=14G --quiet -- <command>`
  (one at a time on this box, memory-capped). Debug `cargo check`/`cargo test` need no wrapper.
- The TUI has no TTY in agent shells: drive it through tmux as the regression scripts do
  (`agent_ergonomics_audit/audit/regression_tests/tui_smoke.sh`). Always `tmux send-keys -l`
  for literal text. Use a unique tmux session name per bead (`fbt-<bead-id>`).
- Never touch the user's live data: tests run with isolated `XDG_*` directories as the harness
  (P0-15) prescribes. Never add, pause, seed or delete the user's real torrents.
- Python 3 and `tmux` are installed. `sudo` is not available.

## Bead workflow (non-negotiable)

1. You work on exactly the bead you were assigned, in the pane you were given. Read it fully:
   `br show <id> --json`. It names the files you own, the API messages, tokens, thresholds,
   fixtures, acceptance criteria and the tests with their JSONL logging.
2. Claiming is the orchestrator's job: your bead arrives already `in_progress` and assigned to
   your pane, with its full text in the marching orders. In a worktree never run `br` at all
   (each checkout would grow its own database and the JSONL would diverge); read the bead text
   you were sent. Never work on a bead you were not sent.
3. Stay inside the bead's owned files. If you must touch a file another bead owns, stop and
   report the exact file and reason instead of editing it.
4. Real code plus real tests in the same change. Acceptance criteria are frozen: never weaken a
   test, never turn a fixture into "live proof", never leave `todo!()`/placeholder code behind a
   passing syntax gate. If a criterion cannot be met, say so in your report and leave it unmet.
5. Run the bead's tests and keep their JSONL logs (`target/test-logs/<bead-id>/...` or the path
   the bead names). Run `cargo test --locked` for anything touching `src/`.
6. Commit when the bead's tests pass: `git add <explicit paths>` (never `-A`), one commit per
   bead (or a few logical commits), message `<area>: <what> (<bead-id>)`, body with the
   acceptance evidence, last line `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
   In a worktree, commit on your branch; the orchestrator merges. Never push, never rebase,
   never `git reset --hard`, `git checkout -- .`, `git clean`, `git stash`, or history rewriting.
7. You do NOT close beads. Leave the bead `in_progress`, and report:
   - bead id, commit SHAs, files changed;
   - each acceptance criterion with its evidence (command, log path, observed result);
   - anything unmet or deviating from the bead, with the reason;
   - follow-ups you noticed but did not do (do not create beads for them).
   The orchestrator verifies on `main` and closes with evidence.
8. Never edit `.beads/` by hand and never commit anything under `.beads/`. Never create or
   delete beads. Follow-ups go in your report.

## Honest credit

Process artifacts are not progress. A receipt, a log, or a document counts only if a named
acceptance criterion requires it. Refusal-only or guard-only changes never close a
positive-capability bead. Commits are not a metric. A report that says "done" without the
evidence rows is treated as not done.

## Where things are

| Area | Path |
|---|---|
| CLI, dispatch | `src/main.rs`, `src/agent.rs` (triage/capabilities/robot-docs) |
| Engine, records, IPC | `src/engine.rs`, `src/ipc.rs`, `src/network_app.rs`, `src/backend.rs`, `src/native_runtime*.rs` |
| Client contract | `src/qbt.rs` (native IPC or legacy WebUI) |
| TUI | `src/tui/mod.rs`, `src/tui/theme.rs`, `src/tui/poster.rs` |
| Search, RSS, Shows, IMDb | `src/search.rs`, `src/browse.rs`, `src/rss.rs`, `src/shows.rs`, `src/imdb.rs` |
| Bar widget | `plugin/oliver.torrents/{manifest.json,Service.qml,Panel.qml,Model.js}` |
| Vendored | `vendor/asupersync` (patched; see `FBITTORRENT_PATCH.md`), `vendor/rqbit`, `vendor/nova3` |
| Tests | `cargo test --locked`; `agent_ergonomics_audit/audit/regression_tests/{all.sh,native.py,tui_smoke.sh,run.sh}`; `scripts/check-*.sh` |
| Plan, decisions | `docs/plan-2026-10-18/PLAN.md` (§9.3 items, Appendix A API, B keymap, E test contract) |
