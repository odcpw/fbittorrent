# fbittorrent — agent-ergonomics scorecard (pass 1, 2026-09-03)

Mode: full. Tier: solo (one worker). Scores are 0–1000 per the 11-dimension rubric;
anything ≥ 700 cites evidence. "Pre" is the CLI at commit 287bf1d, "Post" is the
current HEAD, verified by `audit/regression_tests/run.sh`.

## Surfaces (top 12 by priority)

| surface | dimension | pre | post | evidence (post) |
|---|---|---|---|---|
| bare `fbittorrent` | agent_intuitiveness | 100 | 850 | non-TTY prints `triage --json` (main.rs `run`: `None if … !stdout_is_tty()`); R-002 |
| bare `fbittorrent` | composability | 0 | 850 | TUI never starts without a TTY; `tui` without TTY → exit 2 + hint |
| exit codes (all cmds) | output_parseability | 250 | 900 | 9-code dictionary in `agent.rs::Code`, listed by `capabilities`; R-004/R-005/R-011/R-012 |
| error messages | error_pedagogy | 400 | 900 | every `AppError` carries a copy-pasteable `hint`; JSON envelope `{ok:false,code,code_name,error,hint}`; R-004 |
| `--json` coverage | output_parseability | 500 | 950 | all read + mutating commands emit one JSON object; `emit()` helper; R-007 |
| `add` | agent_ergonomics | 500 | 900 | returns `hash`; `--wait --timeout`; magnet/URL validation with hint; R-012 |
| `wait` (new) | agent_ergonomics | – | 850 | blocks with progress on stderr, exit 8 on timeout, 5 on error state; R-012 |
| `show` (new) | agent_ease_of_use | – | 850 | one torrent + files + `open_target` + `complete`; R-012 |
| `delete --files` | safety_with_recovery | 200 | 950 | requires `--yes`; `--dry-run` previews; hint names both; R-005/R-012 |
| selectors | intent_inference | 450 | 850 | hash prefix OR unique name substring OR `all`; ambiguous → exit 7 listing matches |
| subcommand/flag typos | intent_inference | 700 | 800 | clap suggestions (`lsit` → `ls`, `list`; `--jsno` → `--json`) + aliases ls/rm/pause/resume/find/info/docs; R-006/R-010 |
| `capabilities`, `robot-docs`, `triage` | self_documentation | 0 | 950 | `agent.rs`; `--help` after_help lists exit codes; R-001/R-003/R-009 |
| output bytes | determinism | 800 | 900 | serde_json maps are key-sorted; `capabilities` byte-identical; R-008 |
| everything | regression_resistance | 0 | 800 | 12 tests in `regression_tests/run.sh` |

Median across the audited surfaces: pre ≈ 330, post ≈ 875. No surface regressed.

## Not changed (deliberately)

- The TUI keeps its keyboard contract; ergonomics for humans were handled in the earlier polish passes.
- `search --install` still prints progress-free; installation is a one-time action.
- No colour is emitted anywhere, so `NO_COLOR` is honoured trivially.
