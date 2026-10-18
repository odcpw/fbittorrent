# Round 3 validation of `PLAN.md` (planning-workflow validation loop)

Date: 2026-10-18. Validator: an independent agent that did not write the plan. Scope: the four
checks of the `planning-workflow` skill's "Validation Loop" (self-containment, dependency graph,
justification, grounding) against the round-2 `PLAN.md` (5,075 lines). Coherence and scope are a
sibling review's job and are not repeated here. Nothing under `src/`, `plugin/`, `vendor/` or
`Cargo.*` was touched; no git state command was run; `PLAN.md` was not edited. Scratch files
(extracted items, scripts, verbatim subagent answers, script outputs) are under the session
scratchpad `r3val/`.

## Verdict

The plan does **not yet meet** the planning-workflow "Outcome" bar, on one of the four criteria.

- **Self-contained: fails.** 0 of 8 sampled items were implementable from their own text; 3 of 8
  (P0-08, P1-09, P0-15) were "mostly" implementable once the fresh agent read the section the item
  cites (§3.4.8, §9.1, D2), and 5 of 8 were not implementable even then (P3-13, P4-02, P6-08,
  P7-03, P7-06). The pattern is the same every time: the item is a pointer ("§5.4", "its row in
  §6.3", "the read keys") and the target section sketches rather than specifies. Table-row items
  (P6, P7) are the worst: a fresh agent cannot even find the §6.3 row because the row does not
  name it. Round 1 and 2 self-checked P5-06, P2-08, P5-07 and P1-19, which are among the best
  described items; the sample here was chosen by length and by phase and shows the median.
  Fraction passed: **0/8 as written, 3/8 with the cited section**.
- **Dependency-aware: passes with corrections.** No cycle, no undefined ID, §9.3 and §9.4 agree
  edge for edge, every phase item is an ancestor of its milestone. But 14 items name in their Do or
  Tests something that is defined by an item they neither depend on nor inherit (Section B): the
  theme fixture nobody owns, three TUI-side T1/T2 items that send API v1 operations without
  depending on the TUI's API client (P4-03), the `settings` topic before the bus exists, staged add
  before the stop policy, "Enter stages" views before staging exists. None changes a milestone
  weight; three items move level. The critical-path prose names P1-07 as an L item (it is M) and
  counts nodes as items.
- **Justified: passes, thinly.** Of 12 sampled decisions, 5 carry a usable why, 6 a weak one, 1
  none. The weak ones are all of one kind: numbers (backoff steps, 10 s cap) and borrowings
  (tab set, sources) without the sentence that lets an implementer choose the next number.
- **Grounded: passes.** 40 of 40 sampled `file:line` citations resolve to the claimed content
  (second sweep: 180 more, 5 off by a line or a swap). 10 of 10 numeric claims recompute, except
  one: the §5.5.1 claim that the owner's accent "qualifies" for the bold-foreground fallback is
  false under the rule as written (channel difference 114, ratio 3.3), and P4-05's acceptance
  repeats it, so P4-05's `accent_fallback_table` test would fail on the owner's theme. Every named
  ftui, Quickshell, Omarchy-shell and librqbit API exists in the installed or vendored source
  (Section D).

Recommendation: do one more integration round that (1) expands every P6/P7 table row into the
Do/Why/Accept/Tests/Depends form or makes each row cite its §6.3 row and makes §6.3 carry the
numbers, (2) applies the Section B dependency hunks, (3) fixes the accent-fallback claim and the
five citation slips, then rerun this loop on a new length-sorted sample before beads conversion.

---

## Section A. Self-containment

Method: the eight items were extracted by script (`r3val/extract_items.py`) to one file each,
containing only the item's own text (title, Do, Why, Accept, Tests, Depends, size; for table-row
items, the row with its header). Selection: the two shortest items by character count (P7-03,
104 characters; P7-06, 171), two P0 items with the longest reach (P0-08, P0-15), the P1 API item
whose semantics every client relies on (P1-09), a P3 dashboard view (P3-13), the P4 TUI item that
other items generate from (P4-02), and the P5/P6 feature row with the most surfaces (P6-08). Each
went to a fresh `general-purpose` (opus) subagent with exactly the prescribed prompt and no other
context. Answers are reproduced verbatim (indentation normalised).

### A.1 P7-03 Rename display name (row, 104 characters)

Fresh agent's answer:

> No, not as written. The row says what to do and nothing about where or how. Most of the spec does exist in `docs/plan-2026-10-18/PLAN.md`, but the row never points there, and some of it isn't written down anywhere.
>
> **Things I'd have to look up (and where I found them):**
> 1. **What "display name" means.** It's `Record.name` only, not the payload files. PLAN.md §6.2 row 3 (line 2510), line 2597, and the Out list at line 537.
> 2. **The API call.** `torrents.rename {hash, name: str (1..512)}` returns `{state:"done"}` and is a mutating op. PLAN.md line 4478.
> 3. **The CLI command.** `rename <sel> <name>`. It starts the daemon and raises exit codes 0, 2, 4, 6, 7. PLAN.md line 5042, with the code meanings in appendix A.3.
> 4. **The UI surfaces.** A TUI "prompt" and a plugin "inline edit". PLAN.md line 2597.
> 5. **The storage.** The `Record` struct in `src/engine.rs:38-54`, and the ledger write rules (temp, fsync, rename, dir fsync) at PLAN.md line 1363.
> 6. **What P3-10 and P4-03 deliver.** P3-10 is the details panel. P4-03 moves the TUI to API v1 (`src/tui/store.rs`). PLAN.md lines 1941-1946, 2215 and 4038/4050.
>
> **Things not defined anywhere:**
> - **The name gets overwritten.** Today `src/engine.rs:934` sets `record.name = handle.name()` when an add resolves. Nothing says whether restore, metadata fetch or re-adding a duplicate must leave a renamed name alone. That is the whole substance of "persists across restarts".
> - **What "restart" means:** a daemon restart (exit 75 / idle-exit), a ledger reload, or the offline path.
> - **Reverting:** can you go back to the torrent's own name, and is there an "original name" field?
> - **Validation beyond length:** trimming, control characters, newlines, empty or whitespace-only names.
> - **Whether a rename sends a `torrents` event.**
> - **Whether it works while the daemon is down.** `stop` and `delete` write the ledger directly when it is down (line 664); rename's "starts: yes" means it starts the daemon instead.
> - **`<sel>` against a single `hash`:** what happens when the selector matches several torrents (likely an exit 2 or ambiguity error).
> - **The TUI key, the prompt wording, and where the plugin's inline edit sits** (Overview title?).
> - **The test:** its name, its module, and how it simulates a restart.

Gaps: the row does not cite its §6.3 row, A.5 or Appendix F; the one real design question (a
renamed record must survive `record.name = handle.name()` at `src/engine.rs:934`, which I
verified) is not written anywhere; no key, wording, validation, event or test name. Fix: expand
the row to the item form, and make the §6.3 row carry the storage rule.

```diff
-| P7-03 | Rename display name | Cosmetic | The name persists across restarts (unit) | S | P3-10, P4-03 |
+| P7-03 | Rename display name (§6.3 row "Rename display name") | Cosmetic; the torrent's own name is often a release string | Add `Record.display_name: Option<String>` (serde default `None`; `Torrent.name` serves `display_name` when set, else `name`; `TorrentDetail.original_name` carries the torrent's own name). The restore path and the metadata-resolution path (`src/engine.rs:934`, which today overwrites `record.name`) write `record.name` only and never touch `display_name`. `torrents.rename {hash, name}` (A.5): `name` is trimmed, 1..512 chars, no control characters; an empty string clears the override (revert); the ledger is written with the §3.9 temp-fsync-rename rule and a `torrents` delta `{name}` follows. CLI `rename <sel> <name>` (Appendix F) starts the daemon and refuses an ambiguous selector with exit 7. TUI: `N` on a torrent row opens the prompt "Rename to:" prefilled with the current name; dashboard: the Overview title is an inline `TextField` on `N` or double-click. Tests: `ledger::tests::display_name_survives_restore_and_metadata` (rename, then simulate a daemon restart by reloading the ledger and re-running the metadata path; the override stands), `rename_validation` (table: empty clears, whitespace trimmed, control characters refused with `invalid_args` key `name`), `agent_contract.py::rename_verb` | S | P3-10, P4-03 |
```

### A.2 P7-06 Per-torrent download cap (row, 171 characters)

Fresh agent's answer:

> **Verdict: not implementable from the row alone.** It can be done only after cross-reading PLAN.md, and even then a few gaps remain that I would have to guess.
>
> Facts I would have to guess or look up:
>
> 1. **Cap value and tolerance.** The row doesn't give either. My closest guess is P5-04's `alt_limit_holds`: 64 KiB/s, ±25 %, over 20 s (`docs/plan-2026-10-18/PLAN.md` ~3864). It doesn't say whether warm-up time is excluded, or what the speed is sampled from (daemon `dl_bps` or bytes on disk).
> 2. **Which test counts.** §6.3 (PLAN.md:2600) says the test is "Unit". This row says `daemon.py::download_cap_holds`. They conflict, and nothing says which one wins.
> 3. **API shape.** Appendix A.5 (PLAN.md:4480) gives `torrents.set_download_limit {hashes, bps: bps?} → {state:"done"}`. It doesn't say:
>    - whether there is a minimum like upload's ≥16384 (`src/engine.rs:61-63`, from rqbit reserving a whole chunk);
>    - whether `null` means unlimited;
>    - whether the op is idempotent.
> 4. **Persistence.** The `Record` field name, and whether the cap survives a restart, aren't defined (`src/engine.rs:44` only has `upload_limit`).
> 5. **Precedence.** How the cap combines with the global and alt limits (`min`?) isn't stated.
> 6. **CLI.** For `limit <sel> --down N` (Appendix F, PLAN.md:5040, exit codes 0/2/4/6/7), these are missing:
>    - the unit of N, and whether suffixes like `64K` are accepted;
>    - how to clear the cap;
>    - the JSON output (it is only marked "new").
> 7. **Scope.** §6.3 lists a dashboard "details" control. It's unclear whether that belongs to this item.
> 8. **Capability token.** `download_limit` appears at PLAN.md:1056, but the row doesn't say it is part of the work.
> 9. **Engine support.** P6-07 is supposed to expose live per-torrent `Limits` (`vendor/rqbit/librqbit/src/torrent_state/live/mod.rs:217`). I would need to confirm that the fork actually throttles the download path per torrent, not just upload. `limits.rs` has a `down` limit, but I didn't check that it is used per torrent.
> 10. **Test harness and dependencies.** `daemon.py` doesn't exist yet (it is P1-07), and neither do the local seed fixture or its payload size. P6-07 isn't implemented either.
>
> **To fix the row, add:** the cap value, the tolerance and time window, the CLI unit and how to clear the cap, the minimum bps, the precedence rule, persistence, and a single test of record.

Gaps: as the agent lists; plus the row and its §6.3 row disagree on the test ("Unit" vs
`daemon.py`). The dependency on P3-10 (details action) and P1-11 (the CLI verb) is missing
(Section B).

```diff
-| P7-06 | Per-torrent download cap | Rarely needed beside global limits | A local transfer stays within the cap's tolerance (`daemon.py::download_cap_holds`) | S | P6-07 |
+| P7-06 | Per-torrent download cap (§6.3 row "Per-torrent download cap") | Rarely needed beside global limits; completes the symmetry with the live upload cap | `Record.download_limit: Option<u64>` (bytes per second, persisted; absent = unlimited) applied through the live `Limits` P6-07 exposes, so a change never re-checks. `torrents.set_download_limit {hashes, bps?}`: `bps` ≥ 16384 or `null` for unlimited (the same floor as `torrents.set_upload_limit`, `src/engine.rs:63-65`), idempotent, `{state: "done"}`. Effective rate is `min(per-torrent, global, alt when on)`. `caps` gains `download_limit`. CLI `limit <sel> --down N` accepts bytes per second with `K`/`M` suffixes (`64K`), `--down none` clears, JSON `{ok, action: "limit", hashes, download_limit_bps}`; exit 2 for a value below 16 KiB/s. Dashboard: a field in the Overview tab; TUI: none (T3). Test of record: `daemon.py::download_cap_holds` (the P5-04 fixture: local seed, 64 KiB/s cap, after a 5 s warm-up the leecher's `dl_bps` from `torrents.get` stays within ±25 % over 20 s; a cap change mid-transfer never enters `checking`), `limits::tests::effective_rate_is_min` | S | P6-07, P3-10, P1-11 |
```

And the §6.3 row (L2600) should say the same test:

```diff
-| Per-torrent download cap (P7-06) | uses P6-07's live limits | `torrents.set_download_limit` | none | details | `limit <sel> --down N` | Unit |
+| Per-torrent download cap (P7-06) | uses P6-07's live limits; `Record.download_limit`; effective rate `min(per-torrent, global, alt)` | `torrents.set_download_limit {hashes, bps? ≥ 16384 or null}` | none | details Overview field | `limit <sel> --down N[K\|M]`, `--down none` clears | `daemon.py::download_cap_holds` (±25 % over 20 s after a 5 s warm-up); unit `effective_rate_is_min` |
```

### A.3 P0-08 Graceful shutdown on terminal close (S)

Fresh agent's answer:

> **Verdict: partly implementable. Step (2) rests on a wrong premise.** I didn't modify any files.
>
> **What the code contradicts:**
> - The ftui runtime already checks `pending_termination_signal()` about five times per loop iteration and breaks out of the loop (`ftui-runtime-0.6.0/src/program.rs:5254-5323`). A `Cmd::quit()` on the Hover tick therefore adds nothing, and the proposed unit test would pass without proving any behaviour.
> - The cited lines `program.rs:111-117` are only wrapper functions. The real clear is at `:5490`, inside `complete_lifecycle`, after `shutdown_once`. That means the signal is cleared before `tui::run` returns, so before `drop(app)` at `src/main.rs:819`.
> - So the likely 2.2 s killer is something the plan never names: a second SIGHUP/SIGTERM arriving during the drain, which nobody clears, or a blocking or EIO-erroring call on the dead pty.
>
> **What I'd have to guess:**
> 1. **`FBT_DEBUG_SIGNALS`:** it doesn't exist yet. Output format and destination aren't given, and stderr points at a dead pty.
> 2. **"the log" and "drain messages":** there is no log file. A successful drain prints nothing; only failures `eprintln` (`src/engine.rs:536-537`). The message text and where it goes are undefined.
> 3. **Instance lock:** it is a `flock` on `$XDG_RUNTIME_DIR/fbittorrent/engine.lock` (`src/ipc.rs:131-152`). How the test checks it is "free" (e.g. `flock -n`) isn't stated.
> 4. **`native.py::terminal_close_is_graceful`:** `native.py` is one linear `main()` with no test functions, so it's unclear how to add or select this one. There is no separate daemon in TUI-hosted mode, so "daemon stderr" presumably means `tui.stderr` (`native.py:157`).
> 5. **Watchdog (3):** the model doesn't own `AppEngine`; `main.rs` does. It would need a shared handle that isn't designed. The drain lives inside `Drop` (`src/engine.rs:517-546`) and the socket unlink inside `ipc` `Drop` (`src/ipc.rs:278-286`), so both would need refactoring.
> 6. **Ghostty acceptance:** manual, or automated through `hyprctl`? Unspecified.
> 7. **Unit test:** it must wrap itself in `ftui_core::shutdown_signal::with_test_signal_serialization`, because the pending signal is one process-wide value. The plan doesn't mention this.
> 8. **Exit status:** the expected code after a graceful signal exit (128+sig?) isn't specified.
> 9. **The 6 s bound:** what it's measured from isn't stated (presumably `kill-window`, i.e. 2 s grace + 4 s drain).
>
> **Confirmed correct:** the citations `terminal_session.rs:107,147-167,1270-1295`, `src/tui/mod.rs:1673`, `src/main.rs:819`, the 4 s drain bound, D2 (`PLAN.md:293`) and P0-04 (`PLAN.md:2892`).

I verified the agent's central claim in the pinned crate: `run_event_loop` calls
`observed_termination_signal()` (a wrapper over `check_termination_signal()`) at
`ftui-runtime-0.6.0/src/program.rs:5254,5269,5286,5297,5321,5441`; `complete_lifecycle`
(`:5462`) runs `shutdown_once()` (`:5484`) and only then `clear_termination_signal()` (`:5490`);
`terminal_session.rs:121,151` make `wait_for_shutdown_ack` return true as soon as
`EXPLICIT_TEARDOWN_STARTED` is set (`:1078`). So the runtime already notices the signal without
the model's help, and the 2 s are spent somewhere between the signal thread's record and
`shutdown_once` finishing on a dead pty, or in the engine drop after it. Step (2) as written is
redundant and its unit test proves nothing. Gaps: the instrumentation points, the log
destination (stderr is the dead pty), what "drain messages" are (today a successful drain prints
nothing, `src/engine.rs:536-537` prints only failures), the lock check, the exit status, the 6 s
origin, the test-serialisation wrapper, and that `native.py` has no case structure until P0-15.

```diff
   `ftui_core::shutdown_signal::pending_termination_signal()`; if the runtime does not, it calls
-  `exit(128 + sig)` (`ftui-core-0.6.0/src/terminal_session.rs:107,147-167,1270-1295`; the runtime
-  clears it at `ftui-runtime-0.6.0/src/program.rs:111-117`). The observed 2.2 s death matches the
-  grace. Steps: (1) instrument with `FBT_DEBUG_SIGNALS=1` to log where the 2 s go; (2) in the
-  300 ms `Msg::Hover` tick (`src/tui/mod.rs:1673`) check `pending_termination_signal()` and return
-  `Cmd::quit()` so the loop exits on the next tick, letting the runtime clear the signal and
-  `run_native_app` drop the engine (`src/main.rs:819`); (3) if the runtime cannot clear it in time
-  on a dead pty, spawn a watchdog thread on first sight of the pending signal that drains the
-  engine directly (sets `stopping`, runs the session drain with its 4 s bound, unlinks the socket)
-  and then clears the signal itself. Keep whichever of (2) and (3) the instrumentation shows is
-  needed; the acceptance below is the same.
+  `exit(128 + sig)` (`ftui-core-0.6.0/src/terminal_session.rs:107,147-167,1270-1295`). The
+  runtime's own event loop already polls the pending signal on every iteration
+  (`ftui-runtime-0.6.0/src/program.rs:5254-5441`, through the wrappers at `:110-117`) and clears
+  it only in `complete_lifecycle` **after** `shutdown_once` has restored the terminal
+  (`:5462-5490`); the signal thread also accepts an explicit teardown as acknowledgement
+  (`EXPLICIT_TEARDOWN_STARTED`, `terminal_session.rs:121,151,1078`). So the model needs no
+  `Cmd::quit()` of its own; the 2.2 s are lost between the signal thread's record and that clear,
+  most likely in `shutdown_once` writing to the dead pty, or a second SIGHUP arriving mid-drain.
+  Steps: (1) with `FBT_DEBUG_SIGNALS=1`, append one line per event
+  (`<ms since signal> <event>`) to `$XDG_STATE_HOME/fbittorrent/tui-signals.log` (stderr is the
+  dead pty) for: signal recorded, loop observed it, `shutdown_once` begin/end, clear, `drop(app)`
+  begin/end (`src/main.rs:819`); (2) if the clear happens after the grace, call
+  `clear_pending_termination_signal()` from the model's `Msg::Hover` tick (`src/tui/mod.rs:1673`)
+  as soon as it observes the signal and before returning `Cmd::quit()`, so the signal thread is
+  acknowledged within 300 ms and the engine drop (≤ 4 s, `src/engine.rs:517-546`) runs to the
+  end; (3) if the loop never observes the signal because the input reader blocks on the dead pty,
+  spawn a watchdog thread on first sight of the pending signal that drains the engine directly
+  through a shared `Arc<AppEngine>` handle (sets `stopping`, runs the session drain with its 4 s
+  bound, unlinks the socket) and then clears the signal itself. Keep whichever of (2) and (3) the
+  instrumentation shows is needed; the acceptance below is the same. In both, the engine drop
+  logs `engine drained in <ms> ms` at info level (today `src/engine.rs:536-537` prints only on
+  failure), which is the "drain message" the acceptance looks for. Expected exit status after a
+  graceful signal exit stays `128 + sig` (the runtime's `SignalTerminationError` contract).
 - *Why:* D2. P1 makes the TUI a client, but until then the owner closes the window every day.
-- *Accept:* killing the tmux window or closing the Ghostty window leaves no `engine.sock`, the drain
-  messages appear in the log, and the instance lock is free within 6 s.
-- *Tests:* unit: a test that records a pending signal and asserts the model returns `Cmd::quit()`
-  on the next tick; e2e: `native.py::terminal_close_is_graceful` kills the tmux window with
-  `tmux kill-window`, polls for the socket and the lock, and logs the elapsed time and the daemon
-  stderr tail.
-- *Depends on:* P0-04.
+- *Accept:* killing the tmux window (automated) or closing the Ghostty window (manual, once, logged
+  in the receipt) leaves no `engine.sock`; `engine drained in` appears in `tui.stderr`; the
+  instance lock (`flock` on `$XDG_RUNTIME_DIR/fbittorrent/engine.lock`, `src/ipc.rs:136-164`) is
+  acquirable with `flock -n` within 6 s of `tmux kill-window` (2 s grace plus the 4 s drain bound).
+- *Tests:* unit (wrapped in `ftui_core::shutdown_signal::with_test_signal_serialization`, because
+  the pending signal is one process-wide value): record a pending signal, run one `Msg::Hover`
+  update, assert it is cleared and the model returned `Cmd::quit()`; e2e:
+  `native.py::terminal_close_is_graceful` (a case of the P0-15 harness) kills the tmux window with
+  `tmux kill-window`, polls `engine.sock` and `flock -n engine.lock` every 100 ms, and logs the
+  elapsed time, the exit status and the `tui.stderr` tail (`native.py:157`).
+- *Depends on:* P0-04, P0-15.
```

### A.4 P0-15 Test logging contract and shared harness module (S)

Fresh agent's answer:

> **Verdict: only partly implementable as written.** The Python `Case` and summary parts can be built if I read `docs/plan-2026-10-18/PLAN.md` §9.1 (line 2800: the JSONL example, last 20 lines of tail, the summary table columns, `FBT_TEST_VERBOSE`) and §2.2 principle 10. The `qs` shim and the Rust helper are too vague.
>
> **Things I would have to guess:**
> 1. **JSONL fields.** §9.1 gives only an example line. Is `step` numbered per case? Which timezone does `ts` use? `step(action, expect)` takes no `observed` or `ok`, so how are those set? What exactly does `ms` time?
> 2. **Failure behaviour.** The 11 cases in `native.py` share one TUI session and its fixture state. Does a failing `Case` stop the run or log it and carry on? The case names (slugs) aren't given either.
> 3. **`<case>.daemon.log`.** `native.py` doesn't start a separate daemon. The TUI starts the engine itself and writes one `tui.stderr` for the whole run (`native.py:157`). How do I split one long-running process's stderr into per-case files?
> 4. **"One line per pane capture".** `capture()` is called inside the `wait_for` polling loops. Should every poll be logged?
> 5. **Paths and reruns.** Is `target/test-logs/` relative to the repo root or the current directory? Is it cleared between runs?
> 6. **Forcing a failure.** Nothing says how to trigger one for the acceptance check (an env var, a flag?).
> 7. **`qs` shim.** No name, location, input, or exit-code rule. Which fields does `FBTQS-STEP {json}` carry? How are `FBTQS-PASS`/`FAIL` handled? No QML tests exist until P2-02/P2-06, and no test is listed for the shim.
> 8. **Rust helper.** There is no `tests/` folder and no `src/lib.rs`, and `Cargo.toml` has no dev-dependencies (no `tempfile`, no time crate for ISO 8601). A file under `tests/common/` needs a host test file to run its unit test, and none is named.
> 9. **`scripts/testlog-summary.py`.** `scripts/` doesn't exist. What counts as a failed case (any `ok:false`)? What exit code? Does it go into `all.sh`? §9.1 says it should, but the item doesn't.
> 10. **`fbtlog_test.py`.** Where does it live, what runs it (pytest, unittest, a plain script), and is it wired into `all.sh`?
> 11. **Scope.** Is `FBT_TEST_VERBOSE=1` part of this item?
> 12. **Dependency.** P0-04 (PLAN.md:2892) isn't merged yet. All the target files are untracked, and git commands are restricted until it lands.

Gaps: Appendix E.1 answers part of 1 (`step` increments per case, `final: true`), but the item
cites §9.1, not E.1, and neither gives the `observed`/`ok` mechanics, the failure policy, the
`daemon.log` slicing for a single long-lived process, the shim's name, the Rust host test, the
summary's exit code, or how a failure is forced. Every later e2e test in the plan is written
against this contract, so these have to be fixed here.

```diff
 **P0-15 Test logging contract and shared harness module** (S)
-- *Do:* `agent_ergonomics_audit/audit/regression_tests/fbtlog.py`: `Case(suite, name)` context
-  manager that writes the JSONL lines of §9.1, captures a child process's stderr to
-  `<case>.daemon.log`, prints the tail of both on failure, and exposes `step(action, expect)` with
-  timing; `tests/common/log.rs` with the same shape for Rust integration tests; a `qs` shim that
-  turns `FBTQS-STEP` lines into the same files. Convert `native.py` to it (its 11 `ok:` prints
-  become cases). `scripts/testlog-summary.py` builds the end-of-run table.
+- *Do:* `agent_ergonomics_audit/audit/regression_tests/fbtlog.py`: `Case(suite, name)` context
+  manager that writes the JSONL lines of §9.1 and Appendix E.1 to
+  `<repo>/target/test-logs/<suite>/<case>.jsonl` (`<repo>` is the parent of
+  `agent_ergonomics_audit/`, never the current directory; the suite directory is emptied when the
+  suite starts). `step(action, expect)` returns a step handle whose `ok(observed)` or
+  `fail(observed)` closes it; `ms` is the wall time between `step()` and the close; `step`
+  numbers from 1 per case; `ts` is UTC ISO-8601 with milliseconds and a `Z`. A failing step marks
+  the case failed, is logged with `ok: false`, and the case continues to its end (so one failure
+  shows every later symptom); the suite exits non-zero if any case failed. Case slugs are
+  snake_case of today's `ok:` labels (`dec_mouse_mode`, `closed_status_stays_closed`,
+  `one_enter_adds_once`, `refresh_preserves_read_history`, `rss_mouse_and_scoped_shortcuts`,
+  `mouse_selection_at_scroll_offset`, `native_cli_fields_and_deletion_safety`,
+  `shows_inbox_and_targets`, `file_modal_shows_help_80x24`, `settings_queue_vpn_restart`,
+  `quit_restart_down`; `native.py:172-376`). `Case` captures a child process's stderr into one
+  suite-wide file and records the byte offset at each case boundary, so `<case>.daemon.log` is the
+  slice for that case (`native.py` has one `tui.stderr` for the whole run, `native.py:157`; the
+  slice is what gets printed on failure). Only the capture a step asserts on is logged; polling
+  captures inside `wait_for` are logged only under `FBT_TEST_VERBOSE=1`, which also mirrors every
+  line to stderr (both are part of this item). `FBT_TEST_FORCE_FAIL=<suite>/<case>` makes that
+  case's first step fail, for the acceptance check and for `fbtlog_test.py`. The `qs` shim is
+  `plugin/oliver.torrents/tests/qs-log.py`: it runs `qs -p <dir>` with the given arguments, turns
+  each `FBTQS-STEP {json}` stdout line (same field names as §9.1) into a JSONL line, turns
+  `FBTQS-PASS <case>` / `FBTQS-FAIL <case> <diff>` into the case's `final` line, and exits 1 on any
+  FAIL; it is built here and first used by P2-02. `tests/common/log.rs` writes the same lines
+  with std only (no new dependency; the timestamp comes from `SystemTime` formatted by hand) and
+  is exercised by a host integration test `tests/log_contract.rs` (the `tests/` directory is
+  new). Convert `native.py` to `fbtlog.py` (its 11 `ok:` prints become the cases above).
+  `scripts/testlog-summary.py` (the `scripts/` directory is new) reads `target/test-logs/`,
+  prints the table of §9.1 (suite, cases, passes, failures, slowest five) and exits 1 when any
+  case has `ok: false`; `all.sh` calls it last, in this item (P0-06 adds the CI upload).
 - *Why:* principle 10; the user asked for logging that lets a reviewer see what every test did.
-- *Accept:* `native.py` produces `target/test-logs/native/<case>.jsonl` for each step with at least
-  one line per CLI call or pane capture; a forced failure prints the paths and tails.
-- *Tests:* `fbtlog_test.py`: a fake case that fails writes the expected lines and the summary
-  script counts it; the Rust helper has a unit test writing to a temp dir.
+- *Accept:* `native.py` produces `target/test-logs/native/<case>.jsonl` for each of the 11 cases
+  with at least one line per CLI call or asserted pane capture; `FBT_TEST_FORCE_FAIL=native/quit_restart_down`
+  prints both file paths and their last 20 lines and makes `all.sh` exit non-zero; the summary
+  table lists 11 cases.
+- *Tests:* `regression_tests/fbtlog_test.py` (plain `python3`, called by `all.sh`): a fake suite
+  with one passing and one forced-failing case writes the expected lines (`step` 1..n, `final`
+  verdicts, `ms` ≥ 0) and `testlog-summary.py` counts one failure and exits 1;
+  `tests/log_contract.rs` writes to a temp dir under `std::env::temp_dir()` and asserts the same
+  shape; `qs-log.py --self-test` feeds itself three canned `FBTQS-*` lines.
 - *Depends on:* P0-04.
```

### A.5 P1-09 Mutations: refs, pending, confirmed (M)

Fresh agent's answer:

> Mostly implementable. §3.4.8, A.3/A.5/A.8, §3.2.11 and §4.4 rule 6 (all in `docs/plan-2026-10-18/PLAN.md`) give the shapes. These gaps remain:
>
> **Sequencing conflict**
> - The acceptance "CLI no longer diffs lists" needs the CLI on `api::Client`. That move is P1-11, which depends on P1-09. Either P1-09 pulls part of P1-11 forward, or the legacy-path CLI has to change. The plan doesn't say which.
> - The cited lines have drifted: the URL-path diff runs from `src/main.rs:550` to `559`, not 550-556.
>
> **Ring semantics left unstated**
> - What the cap counts: it says "256 per `(client.name, ref)` pair", but the §3.4.10 limits table says "256 per client name".
> - What gets evicted when the ring is full inside the 10-minute window, and whether an evicted ref is re-applied.
> - Whether the same ref with a different `op` or different `args` is an error or a replay.
> - What a replay returns while the first call is still pending: the stored `pending` response, or a wait.
> - Whether error responses are stored.
> - How the expiry clock is injected for `ring_expiry`.
> - The format of server-generated refs.
> - §3.2.11 wants a pre-crash `remove` ref refused with `invalid_state`. A new instance has an empty ring, so it can't tell a pre-crash ref from a fresh one. Parsing `<start-millis>` out of the ref would be a guess.
>
> **Shapes left unstated**
> - The `mutation.result.result` payload for `add` (`{hash, name, duplicate?}`?).
> - The `pending` response for a URL add when there is no hash yet.
> - Who receives `mutation.result`: the originating connection only, anyone with the same client name after a reconnect, or the `events` topic (A.7, which has no owning item).
> - When `outcome: timeout` fires (the 95 s add timeout?).
> - The error code and `details.key` for more than 256 hashes or an empty list. It is also unclear which ops the cap covers: A.5 bounds `start` to 1..256 but leaves `stop` and `remove` unbounded.
> - `limits.hashes` appears in A.2 but not in the §3.4.3 example.
> - How the info hash is normalised for base32 `btih` and v2 `btmh` magnets.
> - How a URL is fetched: by the daemon or by `AddTorrent::from_url`, with what size cap, and whether a URL that resolves to an existing torrent counts as a duplicate.
> - §3.2.4 says Appendix A marks which ops are idempotent; it has no such column.
>
> **Locations to guess**
> - The `mutations` module isn't in the §3.3 layout; I'd guess `src/api/mutations.rs`.
> - The fixture is `contracts/ipc/v1/mutations.ndjson`, created by P1-05.
> - `daemon.py`, the fixture server and `src/api/` don't exist yet; they come from P1-06 and P1-07.

Gaps: the agent is right on every point; the §3.2.11 "refuse a pre-crash `remove`" rule has no
mechanism (an empty ring cannot recognise a pre-crash ref), §3.4.8 and §3.4.10 disagree on what
256 counts, and the idempotent set is named in §3.2.4 but marked nowhere in Appendix A. The
drift `550-556` → `550-560` is confirmed (`src/main.rs:550` takes the `before` set, `:557-560`
finds the new hash). Hunks: the item, §3.4.8's cap sentence, A.1's request envelope (an
`instance` field is the only way the daemon can tell a resend from a fresh request), and the
acceptance line that belongs to P1-11.

```diff
 **P1-09 Mutations: refs, pending, confirmed** (M)
-- *Do:* §3.4.8: the ref ring keyed by client name; `done` and `pending`; `mutation.result`;
-  mandatory `delete_files`; the 256-hash cap; `torrents.add` returns the hash; duplicate adds
-  return the existing hash with `duplicate: true`.
+- *Do:* §3.4.8 in `src/api/mutations.rs` (add it to the §3.3 layout): the ref ring, 256 entries
+  per `client.name`, each entry keyed by `(client.name, ref)`, oldest evicted first, every entry
+  expiring 10 minutes after it was stored (`Ring::with_clock(fn() -> Instant)` so
+  `ring_expiry` can advance time); an evicted or expired ref is a fresh request. A ref seen again
+  with a different `op` or different `args` is `invalid_args` with `details.key = "ref"` and the
+  message "ref reused for a different request". The stored response is whatever was answered,
+  including `pending` and errors (a client that wants to retry a failed mutation picks a new
+  ref); a replay while the first call is still pending returns the stored `pending` response and
+  the later `mutation.result` reaches the replaying connection too. Server-generated refs are
+  `srv-<first 8 hex of instance>-<counter>`. `done` and `pending`; `mutation.result {ref, op,
+  outcome, error?, result?}` goes to the connection that sent the request, to any connection that
+  has since replayed the same `(client.name, ref)`, and to the `events` topic; `outcome: timeout`
+  fires when the operation's own timeout passes (§3.4.10: 95 s for `torrents.add`, 15 s default;
+  `recheck`, `remove` with files and `move` have no timeout and always end `done` or `failed`).
+  For `torrents.add`, `result` is `{hash, name, duplicate?}`; a URL add answers `{state:
+  "pending", ref}` with no `hash`. Requests may carry `instance: str?` (A.1); a mutation whose
+  `instance` is not the current one is applied as a fresh request when the op is idempotent (the
+  §3.2.4 step 6 list, which A.4 and A.5 mark with `Idem`) and refused with `invalid_state`
+  ("daemon restarted since this request; check the list") for `torrents.remove`,
+  `torrents.recheck` and `torrents.move`; this is the mechanism behind §3.2.11. Mandatory
+  `delete_files`. Every `hashes: [hash]` argument in A.5 is bounded 1..256; an empty list or
+  more than 256 is `invalid_args` with `details.key = "hashes"`; `limits.hashes` in `hello` says
+  256. `torrents.add` returns the hash at once for magnets (a base32 `btih` is decoded to 40
+  lowercase hex; `btmh` v2 magnets are `unsupported`) and `.torrent` paths; `{url}` sources are
+  fetched by the daemon with the §3.4.1 file rules (16 MiB, bencoded, 95 s) and a URL that
+  resolves to a hash already in the ledger answers `duplicate: true` through `mutation.result`.
+  Duplicate adds return the existing hash with `duplicate: true` and change nothing.
 - *Why:* no optimistic lies, no double removes, no hash-diff race in the CLI.
 - *Accept:* repeating a `torrents.remove` ref returns the first result and removes nothing more;
   two clients using the same ref string do not collide; `torrents.remove` without `delete_files` is
-  `invalid_args`; adding a URL gives `pending` and then `mutation.result` with the hash; the CLI no
-  longer diffs lists (`src/main.rs:525-532`, `550-556`); a duplicate add is `ok` with `duplicate`.
+  `invalid_args`; adding a URL gives `pending` and then `mutation.result` with the hash; a reused
+  ref with different args is `invalid_args`; a `remove` carrying a stale `instance` is
+  `invalid_state` while a `stop` with the same stale `instance` applies; a duplicate add is `ok`
+  with `duplicate`. (The CLI's list-diff removal, `src/main.rs:525-532`, `550-560`, is P1-11's
+  acceptance, because the CLI moves to `api::Client` there.)
 - *Tests:* `mutations.ndjson`; `daemon.py::{ref_replay_remove, ref_per_client, remove_requires_flag,
-  add_url_pending_then_result, duplicate_add}`; unit `mutations::tests::ring_expiry`.
+  add_url_pending_then_result, duplicate_add, ref_reused_different_args, stale_instance_remove_refused,
+  stale_instance_stop_applies, hashes_cap_257}`; unit `mutations::tests::{ring_expiry, ring_evicts_oldest,
+  replay_returns_stored_error}`.
 - *Depends on:* P1-06, P1-07.
```

```diff
-- Every mutating operation accepts `ref`. The daemon keeps the last 256 refs per `(client.name,
-  ref)` pair per instance for 10 minutes. Repeating a ref returns the stored response without
+- Every mutating operation accepts `ref`. The daemon keeps the last 256 refs per `client.name`,
+  each keyed by the `(client.name, ref)` pair, per instance, for 10 minutes (P1-09 has the
+  eviction and reuse rules). Repeating a ref returns the stored response without
```

```diff
-- Requests: `{"id": u64 ≥ 1, "op": str, "args": object?, "ref": str? (≤ 64 ASCII)}`. `args` may be
-  omitted when an operation takes none.
+- Requests: `{"id": u64 ≥ 1, "op": str, "args": object?, "ref": str? (≤ 64 ASCII), "instance":
+  str?}`. `args` may be omitted when an operation takes none. `instance` is the daemon instance
+  the client last saw in `hello`; clients send it on every resend after a reconnect (§3.2.4 step
+  6) so the daemon can tell a request that predates its own start (P1-09).
```

### A.6 P3-13 Shows view (M)

Fresh agent's answer:

> **Verdict: not implementable as written.** The item points to other places for almost everything. Most of the answers are in `docs/plan-2026-10-18/PLAN.md`, but several are not written down anywhere.
>
> **Things I had to look up (they do exist):**
> - What [UX] and W2 mean: PLAN.md line 17 and §0 W2 (line 438). The symbols are `●` for unread, plus `↓ … ‖ ✓ +` for transfers (`docs/tui-ux-review.md:150-152`; PLAN §4.3.4).
> - Read keys `m` / `M` / `u` and unfollow `x` / `d`: Appendix B (around line 4685).
> - Badge style, an accent dot plus count on the tab: §4.6 "Top bar".
> - Confirm-dialog rules (`selectedIndex: 0`, `y` maps to `confirmed()`, park the pointer outside the dialog): §4.9.
> - API data: the `shows.*` operations and `Release`/`Show` field shapes in Appendix A.6, A.7 and line 4595. `shows.undo_read` is an API operation, not client state.
> - TUI behaviour: `src/tui/mod.rs:445-510, 1574-1629` (keeping the selection on rebuild, `inbox_last_read`).
>
> **Gaps I would have to guess:**
> 1. **"Step by step" is never defined.** No step script exists. The closest thing is the acceptance tasks 2-3 in [UX] around line 243.
> 2. **The EZTV fixture's contents** (which shows and releases) belong to P3-03's `daemon.py`, which doesn't exist yet. The EZTV URL is hard-coded (`src/shows.rs:547`) and the plan gives no way to point it at a fixture. The fixture-state name for `--serve-fixture` is also missing.
> 3. **The `state()` test hook** (§4.17) has no defined fields for the Shows view, such as selection, read flags or unread counts.
> 4. **There is no dashboard Shows mockup.** The layout, columns, sidebar or pane split, and narrow-width behaviour are all unspecified; only the TUI has one (around line 4883).
> 5. **No QML glyph for unread.** §4.3.4 has no unread row and only gives the TUI `●`.
> 6. **Dashboard Enter on a release** is unclear: download directly with `shows.grab`, or stage it in the add sheet as W1 does? P3-13 doesn't depend on P3-11.
> 7. **Space in Shows**: Appendix B gives the TUI `m`/Space for toggling read, but §4.12 says "Space pauses everywhere".
> 8. **Scope**: are follow (`n`/`+`), check (`r`/`R`) and source (`O`) part of P3-13?
> 9. **Unfollow dialog wording** is not specified.
> 10. **Transfer states missing from the enum**: `transfer` has no stalled, metadata, error or VPN values.
> 11. **Keymap contradictions**: the TUI mockup footer says "(M)ark read", which reads like `M` while the action is `m`. W2 puts quality on `Q`; Appendix B puts it on `=`.

Gaps: all eleven stand. Two are plan-internal contradictions I confirmed: W2 (L440) and P4-07
(L3772) still say `Q` for show quality while Appendix B, §5.7 row 12 and DL-34 say `=`; §4.12
says Space pauses "in every surface" while Appendix B gives Space "toggle read" on Shows and
RSS. `src/shows.rs:547` hard-codes `https://eztv.re/api/get-torrents`, so P3-03's fixture needs
an override the plan never names. Hunks: the item, the W2 and P4-07 keys, the §4.12 sentence, an
unread row in §4.3.4.

```diff
 **P3-13 Shows view** (M)
-- *Do:* Inbox and per-show history; the read keys; the unread and transfer symbols
-  ([UX]); unread badges; a `ConfirmDialog` for unfollow with Cancel focused.
+- *Do:* `views/ShowsView.qml`: a two-pane layout (shows list `Style.space(260)`, releases
+  fill; one pane with a breadcrumb below 1,100 px, as the TUI's narrow layout, §5.6) with the
+  TUI's columns (RELEASE key, SHOW, QUAL, SIZE, SEEDS; `Release` fields of A.9) and the inbox
+  ("All unread", newest first) as the first row of the shows list. Unread marker `●` (U+25CF, in
+  the shell font, the glyph the TUI uses; row added to §4.3.4) before the release key; the
+  transfer symbol from the shared state table (`Release.transfer`). Keys per Appendix B: `m` and
+  Space toggle read, `M` marks the show (or the inbox) read, `u` undoes the last read mark
+  (`shows.undo_read`), `r`/`R` check, `n`/`+` follow by name or IMDb id, `x`/`d` unfollow, `O`
+  opens the source page, Enter on a release stages the add through `shows.grab {stage: true}`
+  (the dashboard stages everywhere, §4.8; the TUI downloads directly, W2) and the release is
+  marked read when the staged add is committed. Unread badges: the `Color.accent` dot plus count
+  on the `3 Shows` tab and the sidebar link (§4.6). Unfollow opens `ConfirmDialog` with
+  `selectedIndex: 0`: "Unfollow 'Slow Horses'? Its releases leave the inbox." [Cancel]
+  [Unfollow]. The `state()` hook (§4.17) reports, for this view, `{view: "shows", show, cursor,
+  unread_total, unread_by_show, last_read_undoable}`. Fixture: `daemon.py --serve-fixture shows`
+  serves canned EZTV JSON (two followed shows, four releases, one read) on a local port and
+  starts the daemon with `FBT_EZTV_BASE_URL` pointing at it (P3-03 adds that override of
+  `src/shows.rs:547`).
 - *Why:* W2.
-- *Accept:* the EZTV fixture inbox matches the TUI's semantics step by step.
-- *Tests:* `qs-dashboard.sh::{shows_inbox_semantics, shows_unfollow_cancel_first}`.
-- *Depends on:* P3-03, P3-08.
+- *Accept:* against the `shows` fixture the view reproduces P3-03's `shows_inbox_semantics`
+  sequence step by step: four unread newest first; `m` on one leaves three and the row loses
+  `●`; `M` on the inbox leaves none and the badge disappears; `u` restores exactly the last mark;
+  Enter on a release opens the add sheet and committing it sets `transfer: downloading` and marks
+  it read; `x` opens the unfollow dialog with Cancel focused and Enter cancels.
+- *Tests:* `qs-dashboard.sh::{shows_inbox_semantics (one `state()` read per step above),
+  shows_unfollow_cancel_first, shows_enter_stages}`; `qs-visual.sh dashboard shows` at 1600 and
+  900 px.
+- *Depends on:* P3-03, P3-08, P3-06, P3-11.
```

```diff
-   IMDb id; `Q` (dashboard: the quality dropdown) sets the preferred quality per show (P3-18).
+   IMDb id; `=` (dashboard: the quality dropdown) sets the preferred quality per show (P3-18;
+   `Q` stays quit everywhere, Appendix B).
```

```diff
-  CLI's human output; the Shows quality prompt (`Q`, P3-18's TUI side); the tab bar shows the daemon
+  CLI's human output; the Shows quality prompt (`=`, P3-18's TUI side); the tab bar shows the daemon
```

```diff
-- Space pauses and resumes in every surface. `PanelKeyCatcher` emits `returnRequested` and then
+- Space pauses and resumes on every torrent list in every surface (on Shows and RSS lists it
+  toggles read, Appendix B). `PanelKeyCatcher` emits `returnRequested` and then
```

```diff
 | in your list (Search, B10) | plus | `+` | In your list | n/a | |
+| unread (Shows, RSS) | `●` (U+25CF, in the shell font) | `●` | Unread | n/a | |
```

### A.7 P4-02 One action table for keys, footer and help (M)

Fresh agent's answer:

> Not implementable as written. The item points to §5.4 and Appendix B of `docs/plan-2026-10-18/PLAN.md` (lines 2250 and 4653), and those only sketch the design. These are the things I would have to guess:
>
> 1. **Types.** `KeyChord` and the payloads of `Scope` (`Screen(..)`, `Pane(..)`, `Modal(..)`) are never defined. I would have to check whether ftui 0.6 has its own chord type.
> 2. **Other surfaces.** `run: fn(&mut TuiApp)` only works for the TUI. Nothing says how dashboard-only and popup-only rows (popup `w`/`t`, Enter opens the file) fit in the table.
> 3. **Conditional keys.** There is no encoding for "while the toast shows", for `s`/`f` meaning something else under `engine = "qbittorrent"` until P4-09, or for confirm-prompt scopes (`y`).
> 4. **CLI shape.**
>    - `RobotDocs` only takes a positional `section` today (`src/main.rs:56-60,322-328`).
>    - There is no text format, no `--json` schema and no default `--surface`.
>    - Nothing says whether the contract version or `capabilities` change.
> 5. **`keymap.json`.**
>    - No schema anywhere, and the file does not exist (`plugin/oliver.torrents/` has none).
>    - P3-15 reads it "static until P4-02", but no item creates it.
>    - P4-02 does not depend on P3-15, so it can land first.
>    - How it is generated is unspecified: build.rs, a script, or an update env var like `FBT_UPDATE_GOLDEN`.
> 6. **README table.** `README.md` has no keys table. Its section, columns and start/end markers for the test are all undefined.
> 7. **Footer.**
>    - The two-row layout (§5.3) is not specified: what row 2 holds, the separators, the label format beyond `(U) Seed`, and where `…` goes next to `(?) Help`.
>    - The `priority` values are not given.
>    - The golden needs a name and fixture. The P0-16 harness (`src/tui/tests/`) does not exist yet.
> 8. **`every_handled_key_in_table`.** Nothing says how to list "handled keys" (about 106 `KeyCode::Char` arms in `src/tui/mod.rs`). Once dispatch goes through the table, the test checks nothing.
> 9. **`global_keys_never_rebound` conflicts with the plan.**
>    - Appendix B gives Files `1` = include, which rebinds `1`. It is not said whether modals count as screens.
>    - P4-07 says Shows quality is `Q`, while Appendix B and §5.7 row 12 say `=`.
>    - The owner still has to confirm `U` and `=` (§10.3 Q14).
> 10. **Where the tests live.** `actions::tests` assumes P4-01 created `src/tui/actions.rs`. The §5.3 tree marks that file "(P4-02)".
> 11. **Help.** The grouping and paging layout is undefined (today's help is in `screens/03-help-page*.txt`).

Gaps: all stand. `README.md` indeed has no keys table (grep), `keymap.json` has no schema and no
creating item (P3-15 consumes a "static" one), and `every_handled_key_in_table` is vacuous once
dispatch is table-driven. Hunks: the item, the §5.3 tree note, a schema paragraph in Appendix B.

```diff
 **P4-02 One action table for keys, footer and help** (M)
-- *Do:* §5.4; `fbittorrent robot-docs keys [--json] [--surface tui|dashboard|popup]` prints the
-  keymap; the plugin's `keymap.json` is generated from it.
+- *Do:* §5.4 in `src/tui/actions.rs` (created empty by P4-01). Types: `KeyChord { code:
+  ftui_core::event::KeyCode, mods: ftui_core::event::Modifiers }`; `Scope::{Global,
+  Screen(Screen), Pane(Screen, Pane), Modal(ModalKind), Prompt}`; every `Action` also carries
+  `surfaces: &'static [Surface]` (`Tui`, `Dashboard`, `Popup`) and an optional `surface_note`
+  (the popup's "Enter opens the file"), so dashboard- and popup-only rows (popup `w`/`t`) live
+  in the same table with `run: None`. Conditional keys are `enabled` closures: the undo toast is
+  `Scope::Screen(..)` with `enabled: |a| a.toast.is_some()`; the legacy `s`/`f` toggles are
+  `enabled: |a| !a.cfg.native()` until P4-09 deletes them; `y` in a prompt is `Scope::Prompt`.
+  `fbittorrent robot-docs keys [--json] [--surface tui|dashboard|popup|all]` (default `tui`;
+  `RobotDocs` gains the flags next to its positional `section`, `src/main.rs:56-60,322-328`)
+  prints the Appendix B table in Markdown, or with `--json` the schema of Appendix B's "keymap
+  schema" paragraph. `scripts/gen-keymap.sh` runs `robot-docs keys --json --surface all` into
+  `plugin/oliver.torrents/keymap.json` (P3-15's hand-written first version follows the same
+  schema) and rewrites the README block between `<!-- keys:begin -->` and `<!-- keys:end -->`
+  (added by this item under "Keys"). Footer row 1: enabled actions of the focused scope in
+  `priority` order (10 Enter/primary, 20 open and pause, 30 seed and remove, 40 add and recheck,
+  90 the rest), each as `(key) Label` separated by two spaces, `…` appended when any did not fit,
+  `(?) Help` always reserved at the right; row 2 is the selected item's full name and context
+  (§5.5.4). Help lists every action grouped by scope in Appendix B's order, paged with PgUp/PgDn
+  as today's help pages. Contract: `robot-docs keys` is listed in `capabilities.commands[]`
+  (additive, 1.2.x).
 - *Why:* ends the three-copy drift; one keymap for all surfaces.
 - *Accept:* the invariant tests pass; the footer shows `…` when actions overflow; the README keys
   table and `keymap.json` match the generated output (test).
-- *Tests:* `actions::tests::{every_handled_key_in_table, no_duplicate_key_per_scope (a toast
-  scope is its screen's scope, so `u` and `U` cannot both mean undo and seed there),
-  readme_table_matches, keymap_json_matches, global_keys_never_rebound (`q`, `Q`, `?`, `1`-`4`
-  mean the same on every screen)}`; golden of the 80-column footer with overflow.
+- *Tests:* `actions::tests::{table_covers_appendix_b (every key in `keymap.json` for the TUI
+  surface is in the table and vice versa), no_handler_outside_table (`rg -c 'KeyCode::Char\('
+  src/tui/screens src/tui/modals` is 0: dispatch goes through `actions::dispatch` only),
+  no_duplicate_key_per_scope (a toast scope is its screen's scope, so `u` and `U` cannot both
+  mean undo and seed there), readme_table_matches, keymap_json_matches,
+  global_keys_never_rebound (`q`, `Q`, `?`, `1`-`4` mean the same on every screen; modals are
+  not screens, so Files' `1` is allowed)}`; golden `footer-overflow-80x24.txt` with a fixture
+  state that enables nine actions.
 - *Depends on:* P4-01.
```

```diff
-src/tui/actions.rs          the action table (P4-02)
+src/tui/actions.rs          the action table (created empty in P4-01, filled by P4-02)
```

```diff
 hand-maintained here and in `plugin/oliver.torrents/keymap.json`.
+
+Keymap schema (`robot-docs keys --json`, and `keymap.json`): `{"version": 1, "surfaces":
+{"tui"|"dashboard"|"popup": [{"id": "torrents.remove", "keys": ["d", "x", "Delete"], "label":
+"Remove, keep files", "scope": "screen:torrents"|"global"|"pane:…"|"modal:…"|"prompt",
+"note": str?}]}}`; keys are spelled as in this table (`Ctrl+k`, `Shift+Tab`, `F6`, `Space`,
+`Enter`, `Esc`).
```

### A.8 P6-08 Origin field, filters, bulk remove, undo (row)

Fresh agent's answer:

> **Verdict:** I could build most of it, but not from the row alone. The row is a pointer. The real spec is spread across `docs/plan-2026-10-18/PLAN.md`: §6.3 row (line 2587), §4.9 (2004–2008), §4.6 sidebar (1924), API tables (4457–4464, 4578–4579), Appendix B keys (4680–4681) and CLI table (5031–5032). After reading those, these gaps remain:
>
> 1. **Undo key in the TUI:** §6.3 says `u`, but Appendix B and §5.7 #12 say `U` ("never `u`"). The two contradict each other.
> 2. **Bulk remove scope:** Does `X` only keep files, or can it also delete them? Does one undo bring back the whole batch or one hash? `undo <hash>` and `{restore: hash}` take a single hash.
> 3. **TUI filters:** Which filters define the "filtered set" in the TUI (the `/` text filter, state, origin)? Only the dashboard's "From:" filter is specified.
> 4. **Where undo state lives:** Is it in memory or on disk? Does it survive a daemon restart or idle exit (P1-12) inside the 10 s? Where are the kept `.torrent` and record stored? Today the `.torrent` store is in `src/engine.rs:166`.
> 5. **Origin to sidebar mapping:** How do `clipboard`, `handler`, `agent` and unknown values map onto the four "From:" buckets (Search, RSS, Shows, Manual)?
> 6. **Old records:** What origin do they get by default, `None` or `manual`? `Record` is at `src/engine.rs:38-53`.
> 7. **Total size:** How is it summed when `size` is null (torrents still in `metadata`)?
> 8. **"Checks to `done`":** This assumes the removed torrent was complete. The fixture, the expected state for an incomplete torrent, and the timeout are not given.
> 9. **Over 256 hashes:** No error code is given; I'd guess `invalid_args`.
> 10. **Missing dependency:** The TUI toast needs P4-08 (toasts and notification queue), which "Depends on" doesn't list.
> 11. **Dashboard toast:** There is no QML spec for its placement, text or button.
> 12. **Test assertions:** No assertions or `state()` hook fields are given for `bulk_remove_confirm`/`undo_toast`.
> 13. **Test harnesses don't exist yet:** `daemon.py` comes from P1-07 and `plugin/oliver.torrents/tests/qs-dashboard.sh` from P3-08. Nothing in the repo matches today, and the daemon, API v1 and dashboard aren't built.

Gaps: all stand; the `u`/`U` contradiction in §6.3 L2587 is confirmed. Hunks: the row and the
§6.3 row.

```diff
-| P6-08 | Origin field, filters, bulk remove, undo for keep-files removal | W8 cleanup; grouping without categories (§6.2 rows 21, 32) | Origin persists (unit); a filtered bulk removal shows count and size and needs `y`; undo within 10 s re-adds and checks to `done` (`daemon.py::undo_restore`, `qs-dashboard.sh::{bulk_remove_confirm, undo_toast}`); CLI `undo` | M | P3-09, P4-04 |
+| P6-08 | Origin field, filters, bulk remove, undo for keep-files removal (§6.3 row "Origin, filters, bulk remove, undo") | W8 cleanup; grouping without categories (§6.2 rows 21, 32) | `Record.origin: open(manual\|search\|rss\|shows\|clipboard\|handler\|agent)`, serde default `manual` for old records; the sidebar's "From:" buckets are Search, RSS, Shows and Manual (clipboard, handler, agent and unknown values count as Manual). Bulk remove is keep-files only: `X` acts on the rows currently listed (the active state filter, the `/` text filter and the From filter together, in both UIs) and confirms with the count and the summed `size` of the rows that have one ("+ 2 of unknown size" otherwise); deleting files stays per torrent (`D`) or `delete --files --yes`. Undo state is in memory and dies with the daemon: `torrents.remove {delete_files: false}` moves the record and its `.torrent` (`src/engine.rs:166`) to `engine/undo/<hash>.{json,torrent}` and answers `undo_until`; `torrents.add {source: {restore: hash}}` within the window re-adds from there; `undo` with no hash restores the last batch (`undo_until` is per batch). More than 256 hashes is `invalid_args` key `hashes`. Toast: the dashboard's `Banner` component at the bottom of the list, "Removed 'X' (3 torrents) · U undo", for 10 s; the TUI's P4-08 toast with the same text. Tests: `ledger::tests::origin_default_manual`; `daemon.py::undo_restore` (a completed local fixture is removed and restored: `done` within 10 s, no re-download; an incomplete one restores as `checking` then `downloading`); `daemon.py::bulk_remove_mixed_sizes`; `qs-dashboard.sh::{bulk_remove_confirm (the dialog text names count and size; Enter cancels), undo_toast (`state().toast` names the batch; `U` restores; the toast is gone at 11 s)}`; CLI `undo` in `agent_contract.py::undo_verb` | M | P3-09, P4-04, P4-08 |
```

```diff
-| Origin, filters, bulk remove, undo (P6-08) | `Record.origin` set by search grab, RSS grab, Shows grab, clipboard, handler; undo keeps the `.torrent` file and the record for 10 s after a keep-files removal | `origin` field; `torrents.remove` with up to 256 hashes; `torrents.add {source: {restore: hash}}` within the undo window | `X` on a filtered view with count and size confirm; undo toast `u` | sidebar "From:" filters; bulk remove; undo toast | `delete … --json` returns `undo_until`; `undo <hash>` | Unit: origin persisted; `daemon.py::undo_restore` re-adds and checks to `done` |
+| Origin, filters, bulk remove, undo (P6-08) | `Record.origin` set by search grab, RSS grab, Shows grab, clipboard, handler (default `manual`); undo keeps the `.torrent` file and the record under `engine/undo/` for 10 s after a keep-files removal, in memory, per batch | `origin` field; `torrents.remove` with up to 256 hashes; `torrents.add {source: {restore: hash}}` within the undo window | `X` on a filtered view with count and size confirm; undo toast `U` (never `u`, which is seed) | sidebar "From:" filters; bulk remove; undo toast | `delete … --json` returns `undo_until`; `undo [<hash>]` (no hash: the last batch) | Unit: origin persisted; `daemon.py::undo_restore` re-adds and checks to `done` |
```

### A.9 Cross-cutting fix for the P6 and P7 tables

Five of eight failures are table rows. The P6/P7 intro says "its daemon, API, UI and test details
are its row in §6.3", but a reader of one row cannot find that row (the titles differ:
"Rename display name" vs "Rename display name (P7-03)"), and the §6.3 rows themselves are
one-liners for the T3 items. Minimum fix without rewriting P6/P7 into bullets: every P6/P7 row
names its §6.3 row in the Title cell (as the hunks above do), and every §6.3 row for an In
feature carries the numbers an implementer needs (thresholds, caps, units, the test of record).
The sibling coherence review may prefer converting P6/P7 into the bullet form; either satisfies
the check.

---

## Section B. Dependency graph

Script: `r3val/dagcheck3.py` (independent of the round-2 reviewer's `dagcheck.py`; parses the
§9.3 items from bullets, table rows and milestone lines, expands `a..b`/`a to b` ranges, compares
with the §9.4 adjacency table, runs Kahn, checks the level table, milestones, orphans and
weighted paths). Output:

```
§9.3 ids: 108; §9.4 rows cover 107 ids
NOT IN §9.4: P7-07
levels: 19; nodes 108; no cycle
NOT IN LEVEL TABLE P7-07 (computed 0)
max level 18
orphans (no consumer, not a milestone): ['P7-07', 'P7-08']
M0: phase items not ancestors: []; direct deps: 11; redundant direct deps (already implied): ['P0-16', 'P0-15']
M1: phase items not ancestors: []; direct deps: 10; redundant direct deps (already implied): ['P2-03']
M2: phase items not ancestors: []; direct deps: 2; redundant direct deps (already implied): []
M3: phase items not ancestors: []; direct deps: 7; redundant direct deps (already implied): []
M4: phase items not ancestors: []; direct deps: 5; redundant direct deps (already implied): []
M5: phase items not ancestors: []; direct deps: 13; redundant direct deps (already implied): []
M6: phase items not ancestors: []; direct deps: 7; redundant direct deps (already implied): []
critical path to M1: 17 nodes (15 items), weight 35: M1 <- P2-05 <- P2-03 <- P2-08 <- P2-01 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-02
critical path to M2: 18 nodes (16 items), weight 37: M2 <- P3-16 <- P3-10 <- P3-09 <- P3-08 <- P2-01 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-02
critical path to M3: 19 nodes (17 items), weight 44: M3 <- P4-10 <- P4-04 <- P4-03 <- P4-01 <- P1-15 <- P1-12 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-02
critical path to M4: 18 nodes (16 items), weight 40: M4 <- P5-05 <- P4-03 <- P4-01 <- P1-15 <- P1-12 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-02
critical path to M5: 19 nodes (17 items), weight 44: M5 <- P6-08 <- P4-04 <- P4-03 <- P4-01 <- P1-15 <- P1-12 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-02
critical path to M6: 19 nodes (17 items), weight 44: M6 <- P7-04 <- P6-01 <- P4-03 <- P4-01 <- P1-15 <- P1-12 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-19 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-02
sizes: Counter({'M': 45, 'S': 44, 'L': 11, '-': 8})

--- items whose Do/Accept/Tests text names an item that is neither a dependency nor an ancestor nor a consumer (heuristic):
  P2-01: mentions ['P1-14']
  P3-09: mentions ['P3-17']
  P3-15: mentions ['P4-02', 'P5-04', 'P6-13']
  P5-03: mentions ['P1-16']
  P7-07: mentions ['P1-14']
```

Reading: structurally clean and consistent with round 2 (the "orphan" P7-07 is the superseded
row, correctly absent from §9.4; P7-08 is conditional by design). The level table matches. Every
phase item is an ancestor of its milestone (M0's direct deps P0-15/P0-16 and M1's P2-03 are
redundant but harmless). Weights match the plan's numbers (35/37/44/40/44/44).

### B.1 Edges the Do/Accept/Tests text needs but Depends omits

Checked by `r3val/anc.py` against the transitive closure; "MISSING" means neither a direct
dependency nor an ancestor.

| Item | Needs | Why (text that requires it) | Status |
|---|---|---|---|
| P2-08 | P4-05 | Accept: `qs-format.sh::dim_ladder` asserts `Model.dim` equals `palette.rs` for every fixture theme; `palette.rs` is created empty in P4-01 and filled in P4-05 | MISSING both ways: P4-05 also needs P2-08's `contracts/ui/themes/` + `expected.json`, which no item says it creates |
| P5-03 | P4-03 | `R` in the TUI sends `torrents.recheck`; the compat socket has no such route | MISSING |
| P5-03 | P1-16 | Accept/Tests: `recheck_dormant` (P1-16's dormant torrents) | MISSING |
| P5-04 | P4-03 | `z` and F6 in the TUI send `transfer.set_alt`/`set_limits` | MISSING |
| P6-05 | P4-03 | `P`, `K`/`J` in the TUI send `torrents.force_start`/`queue_move` | MISSING |
| P3-05 | P1-08 | Do: "the `settings` topic"; Accept: clients "receive `engine.changed`" (both P1-08) | MISSING (P3-05 depends on P1-06 only) |
| P3-06 | P1-12 | Do: staged records excluded from the stop policy; Test `staged_not_busy` | MISSING |
| P3-11 | P3-05 | Do: the `handler = "add" \| "confirm"` setting is a `Settings` field P3-05 adds | MISSING |
| P3-12, P3-14 | P3-06 | Do: "Enter stages the add" | MISSING |
| P3-18 | P3-07 | Do: CLI `shows quality <show> <q>` goes through the daemon with fallback | MISSING |
| P5-05, P5-07 | P2-04 | Do: the bar icon state (ready / urgent) is P2-04's icon table | MISSING |
| P6-08 | P4-08 | the TUI undo toast is P4-08's `toast`/`notification_queue` | MISSING |
| P7-06 | P3-10, P1-11 | details action; CLI verb | MISSING |
| P2-01 | (P1-14) | Tests: "reads the hello counters (P1-14)" | MISSING; fix the text instead: `app.info.clients` exists since P1-06 (A.4) |
| P3-09 | (P3-17) | Accept: "`QSG_RENDER_TIMING=1` log (the mechanism of P3-17)", a forward reference to a later item | define the mechanism in P3-09 |
| P0-07, P0-08, P0-09, P0-11, P0-14 | P0-15 | each names a `native.py::<case>`; cases exist only after P0-15 converts `native.py` | MISSING (optional but consistent) |
| P2-07 | (P3-08) | §8.6: `torrents.open` summons the dashboard, which does not exist at v0.2.0 | fix the text: fall back to the TUI until P3-08 |

Not flagged (checked and transitive): P1-13←P1-04, P1-15←P1-06, P4-07←P4-03, P4-09←P1-10,
P6-08←P1-09, P3-08←P1-04.

Recomputed with the edges above (`r3val/recompute.py`): no cycle; no milestone weight changes
(M1 35, M2 37, M3 44, M4 40, M5 44, M6 44); levels change for P0-07/08/09/14 (2→3), P3-05
(10→12), P3-06 (12→13), P7-06 (9→16); 18 levels remain.

### B.2 Prose errors in the critical-path text

- §9.2 (L2836-2837) and §9.4 (L4129-4130) call P1-07 an L item in "four L items in series
  (P1-03, P1-06, P1-07, P1-08)". P1-07 is **M**. The four L items on the weight-35 path are
  P1-03, P1-06, P1-08 and P2-03.
- §9.4 (L4120) says "17 items deep": the path has 17 nodes, of which 15 are items and two are
  milestones (`review-log.md` says 16, a third count). Say "17 nodes (15 items)".
- The ASCII diagram at L4124-4126 draws the main line through `P1-08 ─► P1-12 ─► P2-03`
  (weight 33) and the actual critical branch `P2-01 ─► P2-02/P2-08` as the side line; the 35
  path is `P1-08 → P2-01 → P2-08 → P2-03`. Cosmetic; swap the two lines when the diagram is next
  touched (not hunked here because of the box-drawing widths).

### B.3 Hunks

Depends lines (§9.3):

```diff
-- *Depends on:* P2-01, P1-07.
+- *Depends on:* P2-01, P1-07. (P2-08 creates `contracts/ui/themes/` and `expected.json`, computed by `contrast.py`; the cross-check against `palette.rs` is P4-05's `dim_matches_fixture`, because `palette.rs` does not exist before P4-05.)
```

```diff
   reason off Omarchy; the fixture sweep always runs).
-- *Depends on:* P4-01.
+- *Depends on:* P4-01, P2-08 (the theme fixture and `expected.json`).
```

(The hunk above applies to P4-05's Depends line at L3761; P4-02's identical line at L3727 stays.)

```diff
-- *Depends on:* P1-09, P3-10, P4-02.
+- *Depends on:* P1-09, P3-10, P4-02, P4-03 (the TUI sends `torrents.recheck` over API v1), P1-16 (`recheck_dormant`).
```

```diff
-- *Depends on:* P3-05, P2-03, P3-09, P4-02.
+- *Depends on:* P3-05, P2-03, P3-09, P4-02, P4-03 (F6 and `z` in the TUI set limits over API v1).
```

```diff
   settings_restart_class_keeps_clients}`.
-- *Depends on:* P1-06.
+- *Depends on:* P1-06, P1-08 (the `settings` topic and `engine.changed`).
```

(P3-05, L3554; P3-04's identical line at L3543 stays.)

```diff
-- *Depends on:* P1-09.
+- *Depends on:* P1-09, P1-12 (staged torrents must not count as busy).
```

```diff
-- *Depends on:* P3-09, P3-06.
+- *Depends on:* P3-09, P3-06, P3-05 (the `handler` setting).
```

```diff
-- *Depends on:* P3-01, P3-04, P3-08.
+- *Depends on:* P3-01, P3-04, P3-08, P3-06 (Enter stages).
```

```diff
-- *Depends on:* P3-02, P3-08.
+- *Depends on:* P3-02, P3-08, P3-06 (Enter stages).
```

```diff
-- *Depends on:* P3-13.
+- *Depends on:* P3-13, P3-07 (the CLI verb goes through the daemon with the in-process fallback).
```

```diff
-- *Depends on:* P5-06, P1-13, P3-10, P4-03, P2-03, P1-18.
+- *Depends on:* P5-06, P1-13, P3-10, P4-03, P2-03, P1-18, P2-04 (the bar icon's ready state).
```

```diff
-- *Depends on:* P1-08, P2-03, P3-09, P4-03 (the TUI as an API v1 client renders `Summary.disk`
-  and the `! Disk` row; the banner slot already exists in §5.6's layouts).
+- *Depends on:* P1-08, P2-03, P3-09, P4-03 (the TUI as an API v1 client renders `Summary.disk`
+  and the `! Disk` row; the banner slot already exists in §5.6's layouts), P2-04 (the bar icon's
+  urgent state).
```

```diff
-| P6-05 | Force start and queue order | "Watch this one now" when the slots are full (§6.2 row 20) | Unit tests of queue admission with forced and reordered records; `daemon.py::{force_start_bypasses_limit, queue_move_order}`; CLI `start --force`, `queue` | M | P1-09, P3-09, P4-02 |
+| P6-05 | Force start and queue order | "Watch this one now" when the slots are full (§6.2 row 20) | Unit tests of queue admission with forced and reordered records; `daemon.py::{force_start_bypasses_limit, queue_move_order}`; CLI `start --force`, `queue` | M | P1-09, P3-09, P4-02, P4-03 |
```

P0 items that name `native.py` cases (P0-07 L2941, P0-08 L2965, P0-09 L2980, P0-14 L3037 read
`- *Depends on:* P0-04.`; P0-11 L3003 reads `- *Depends on:* P0-04, P0-16.`): append `, P0-15`
to each. (Not hunked line by line because `- *Depends on:* P0-04.` occurs five times in P0; the
integrator applies it to those four items and to P0-11's line; P0-08's version is already in the
A.3 hunk.)

Text-only fixes:

```diff
-- *Tests:* `daemon.py::bar_single_connection` reads the hello counters (P1-14) while the live
+- *Tests:* `daemon.py::bar_single_connection` reads `app.info`'s `clients.bar` count (A.4, P1-06) while the live
```

```diff
 - *Accept:* with 200 fixture torrents, 10 s of 1 Hz deltas plus continuous wheel scrolling
-  produce no frame over 32 ms in the test instance's `QSG_RENDER_TIMING=1` log (the mechanism of
-  P3-17) and the "no JavaScript loop per delta" lint passes; every §4.11 state; Appendix B keys;
+  produce no frame over 32 ms in the test instance's log: `qs-dashboard.sh` starts the test
+  shell with `QSG_RENDER_TIMING=1` and parses the per-frame `render` lines Qt Quick prints to
+  stderr (`time in renderer: … ms`), taking the slowest; P3-17 reuses this. The "no JavaScript
+  loop per delta" lint passes; every §4.11 state that exists at P3 (the P5/P6 rows wait for their
+  items); Appendix B keys;
```

```diff
   README install section.
+  Until P3-08 adds the `panel` kind, `torrents.open` and `torrents.search` run `fbittorrent
+  dashboard`, which falls back to the TUI when the plugin manifest has no `panel` kind
+  (Appendix F), so the v0.2.0 menu never points at a window that does not exist.
```

```diff
-files. The weighted critical path to M1 runs through four L items in series (P1-03, P1-06,
-P1-07, P1-08; §9.4); the plugin lane can start `ApiClient.js`, `Store.js` and the P2-08 harness
+files. The weighted critical path to M1 runs through four L items (P1-03, P1-06, P1-08 and
+P2-03, with the M-sized P1-07 between them; §9.4); the plugin lane can start `ApiClient.js`, `Store.js` and the P2-08 harness
```

```diff
-Critical path to the first demo (17 items deep, weight 35 in S-units with S=1, M=2, L=4;
+Critical path to the first demo (17 nodes deep: 15 items and two milestones; weight 35 in S-units with S=1, M=2, L=4;
```

```diff
-Weighted longest paths (the script's output, round 2): M1 35 (through P1-03, P1-06, P1-07,
-P1-08 in series: four L items), M2 37 (… P2-01 → P3-08 → P3-09 → P3-11 → P3-16), M3 44 (… P1-12
+Weighted longest paths (the script's output, round 2): M1 35 (through the L items P1-03, P1-06,
+P1-08 and P2-03, with P1-07 (M) between them), M2 37 (… P2-01 → P3-08 → P3-09 → P3-11 → P3-16), M3 44 (… P1-12
```

§9.4 adjacency rows (old → new; consumed-by columns updated to match):

```diff
-| P0-07, P0-08, P0-09, P0-12, P0-14 | P0-04 | M0 |
-| P0-10, P0-11, P0-13 | P0-04, P0-16 | M0 |
-| P0-15 | P0-04 | P0-06, P1-07, M0 |
+| P0-12 | P0-04 | M0 |
+| P0-07, P0-08, P0-09, P0-14 | P0-04, P0-15 | M0 |
+| P0-10, P0-13 | P0-04, P0-16 | M0 |
+| P0-11 | P0-04, P0-15, P0-16 | M0 |
+| P0-15 | P0-04 | P0-06, P0-07, P0-08, P0-09, P0-11, P0-14, P1-07, M0 |
```

```diff
-| P1-08 | P1-06, P1-07 | P1-12, P1-13, P1-14, P1-18, P2-01, P3-01, P3-02, P3-03, P5-01, P5-07, P6-01..P6-04 |
+| P1-08 | P1-06, P1-07 | P1-12, P1-13, P1-14, P1-18, P2-01, P3-01, P3-02, P3-03, P3-05, P5-01, P5-07, P6-01..P6-04 |
```

```diff
-| P1-11 | P1-04, P1-09 | P1-18, P2-07, P3-07 |
+| P1-11 | P1-04, P1-09 | P1-18, P2-07, P3-07, P7-06 |
```

```diff
-| P1-12 | P1-08 | P1-15, P1-17, P2-03, P3-02, P3-03 |
+| P1-12 | P1-08 | P1-15, P1-17, P2-03, P3-02, P3-03, P3-06 |
```

```diff
-| P1-16 | P1-19, P1-07 | M1, P6-06 |
+| P1-16 | P1-19, P1-07 | M1, P5-03, P6-06 |
```

```diff
-| P2-04 | P2-02, P2-08 | P2-05 |
+| P2-04 | P2-02, P2-08 | P2-05, P5-05, P5-07 |
```

```diff
-| P2-08 | P2-01, P1-07 | P2-03, P2-04, P3-09 |
+| P2-08 | P2-01, P1-07 | P2-03, P2-04, P3-09, P4-05 |
```

```diff
-| P3-05 | P1-06 | P3-15, P5-04, P6-09, P6-13 |
-| P3-06 | P1-09 | P3-11 |
+| P3-05 | P1-06, P1-08 | P3-11, P3-15, P5-04, P6-09, P6-13 |
+| P3-06 | P1-09, P1-12 | P3-11, P3-12, P3-14 |
```

```diff
-| P3-07 | P3-01, P3-02, P3-03, P1-11 | M2, P4-03 |
+| P3-07 | P3-01, P3-02, P3-03, P1-11 | M2, P3-18, P4-03 |
```

```diff
-| P3-10 | P3-09, P3-04, P1-14 | P3-16, P5-02, P5-03, P5-05, P6-01, P6-02, P7-02, P7-03 |
-| P3-11 | P3-09, P3-06 | P3-16, P6-11 |
-| P3-12 | P3-01, P3-04, P3-08 | P3-16, P3-17, P7-05 |
+| P3-10 | P3-09, P3-04, P1-14 | P3-16, P5-02, P5-03, P5-05, P6-01, P6-02, P7-02, P7-03, P7-06 |
+| P3-11 | P3-09, P3-06, P3-05 | P3-16, P6-11 |
+| P3-12 | P3-01, P3-04, P3-08, P3-06 | P3-16, P3-17, P7-05 |
```

```diff
-| P3-14 | P3-02, P3-08 | P3-16 |
+| P3-14 | P3-02, P3-08, P3-06 | P3-16 |
```

```diff
-| P3-18 | P3-13 | P3-16, P4-07 |
+| P3-18 | P3-13, P3-07 | P3-16, P4-07 |
```

```diff
-| P4-03 | P4-01, P3-07 | P4-04, P4-08, P4-09, P5-02, P5-05, P5-07, P6-01, P6-02, P6-04, P7-03 |
+| P4-03 | P4-01, P3-07 | P4-04, P4-08, P4-09, P5-02, P5-03, P5-04, P5-05, P5-07, P6-01, P6-02, P6-04, P6-05, P7-03 |
```

```diff
-| P4-05, P4-06 | P4-01 | M3 |
+| P4-05 | P4-01, P2-08 | M3 |
+| P4-06 | P4-01 | M3 |
```

```diff
-| P4-08 | P4-03, P1-14 | M3 |
+| P4-08 | P4-03, P1-14 | M3, P6-08 |
```

```diff
-| P5-03 | P1-09, P3-10, P4-02 | M4 |
-| P5-04 | P3-05, P2-03, P3-09, P4-02 | M4 |
-| P5-05 | P5-06, P1-13, P3-10, P4-03, P2-03, P1-18 | M4 |
+| P5-03 | P1-09, P3-10, P4-02, P4-03, P1-16 | M4 |
+| P5-04 | P3-05, P2-03, P3-09, P4-02, P4-03 | M4 |
+| P5-05 | P5-06, P1-13, P3-10, P4-03, P2-03, P1-18, P2-04 | M4 |
```

```diff
-| P5-07 | P1-08, P2-03, P3-09, P4-03 | M4 |
+| P5-07 | P1-08, P2-03, P3-09, P4-03, P2-04 | M4 |
```

```diff
-| P6-05 | P1-09, P3-09, P4-02 | M5 |
+| P6-05 | P1-09, P3-09, P4-02, P4-03 | M5 |
```

```diff
-| P6-08 | P3-09, P4-04 | M5 |
+| P6-08 | P3-09, P4-04, P4-08 | M5 |
```

```diff
-| P7-06 | P6-07 | M6 |
+| P7-06 | P6-07, P3-10, P1-11 | M6 |
```

Level table rows (recomputed):

```diff
-| 2 | P0-05, P0-07, P0-08, P0-09, P0-12, P0-14, P0-15, P0-16 |
-| 3 | P0-06, P0-10, P0-11, P0-13 |
+| 2 | P0-05, P0-12, P0-15, P0-16 |
+| 3 | P0-06, P0-07, P0-08, P0-09, P0-10, P0-11, P0-13, P0-14 |
```

```diff
-| 9 | P1-04, P1-06, P1-10, P7-06 |
-| 10 | P1-07, P3-04, P3-05 |
+| 9 | P1-04, P1-06, P1-10 |
+| 10 | P1-07, P3-04 |
```

```diff
-| 12 | P1-11, P1-12, P1-13, P1-14, P2-01, P3-01, P3-06, P5-01, P6-06, P7-09 |
-| 13 | P1-15, P1-17, P1-18, P2-02, P2-07, P2-08, P3-02, P3-03, P3-08, P7-01 |
+| 12 | P1-11, P1-12, P1-13, P1-14, P2-01, P3-01, P3-05, P5-01, P6-06, P7-09 |
+| 13 | P1-15, P1-17, P1-18, P2-02, P2-07, P2-08, P3-02, P3-03, P3-06, P3-08, P7-01 |
```

```diff
-| 16 | M1, P3-16, P4-04, P4-08, P5-02, P5-03, P5-04, P5-05, P5-07, P6-01, P6-02, P6-04, P6-05, P6-11, P7-02, P7-03 |
+| 16 | M1, P3-16, P4-04, P4-08, P5-02, P5-03, P5-04, P5-05, P5-07, P6-01, P6-02, P6-04, P6-05, P6-11, P7-02, P7-03, P7-06 |
```

After these edits `scripts/dagcheck.py` must be rerun; the expected output is "no cycle; 107
nodes, 19 levels; orphans ['P7-08']" with the same weights as today.

---

## Section C. Justification

Method: `r3val/paras.py` numbered every paragraph of §2 to §8 (blank-line blocks, each table row
and each list item its own paragraph, code fences excluded): 808 paragraphs, 493 longer than 120
characters. `random.seed(20261018)`; `random.sample` of 12 from the 493. Grades: **Strong** (a
why a future agent can extend), **Weak** (a what with a pointer or a half-sentence), **Missing**.

| ¶ | Lines | Decision | Grade | Rationale found / proposed |
|---|---|---|---|---|
| 57 | 514 | Constraint row: "likes OmaTorrent's format and capabilities" → daemon + popup + dashboard + parity | Weak | Says what honours it, not which format and capabilities. Hunk below names them from [OR §2.1, §4]. |
| 157 | 875-876 | Reconnect backoff 0.5, 1, 2, 4, 8 s, then every 10 s | Missing | No why for any number. Hunk below. |
| 297 | 1317-1319 | Mojibake repair: re-decode only when the name round-trips as UTF-8 misread as Latin-1 | Weak | D15 is the motive; the safety argument (why this heuristic cannot damage a real Latin-1 name) is absent. Hunk below. |
| 405 | 1693-1700 | One state table (`contracts/ui/states.json`) for popup, table, TUI, CLI; `…` stays queued | Strong | Explains the four columns, the width constraint and why `…` keeps its Shows meaning (DL-32 adds the fixture argument). |
| 452 | 1810-1820 | Icon visibility `online \|\| showWhenStopped \|\| root.opened \|\| stoppedRecently` | Strong | Each term has its reason (anchor under an open popup, the 10 s visible exit, the owner's rule, Q2). |
| 479 | 1948-1950 | Details tabs follow qBittorrent's property tabs minus HTTP sources plus Pieces and Log | Weak | Names the source; why Pieces and Log are added is only implicit in §4.7's rows. Hunk below. |
| 681 | 2524 | Remote backend Out; legacy mode removed in v0.4.0 | Strong | "§2.6; SSH plus the TUI covers remote use" and §2.6's "the owner left qBittorrent on purpose". |
| 682 | 2525 | Completion notifications In, T1 | Strong | "Tell me when it is ready to watch; the daemon owns it" ties to W1 and A1. |
| 687 | 2530 | Per-torrent limits: live upload cap T2, download cap T3 | Strong | "Removes a surprising re-check; download caps are rarely needed with global limits." |
| 734 | 2587 | §6.3 row for P6-08 (origin, bulk remove, undo) | Weak | A spec row; the why lives in §6.2 rows 21/32 and W8. Its `u` contradicts Appendix B (fixed in A.8). |
| 738 | 2591 | §6.3 row for P6-12 (verify the fork honours the private flag) | Weak | "Verify … fix if not" without saying why verification is needed. Hunk below, grounded in the fork. |
| 801 | 2756-2758 | `robot-docs` and `capabilities` move to contract 1.2.0 with the listed additions | Weak | What, not why 1.2.0 rather than 2.0.0. Hunk below. |

Score: 5 Strong, 6 Weak, 1 Missing. Hunks for the Weak and Missing rows:

```diff
-| 2026-10-18: likes OmaTorrent's format and capabilities | Daemon + Quickshell popup + dashboard (§3, §4) and the parity work (§6). |
+| 2026-10-18: likes OmaTorrent's format and capabilities | Daemon + Quickshell popup + dashboard (§3, §4) and the parity work (§6). "Format" means a native Omarchy surface fed by a daemon rather than a terminal app [OR §2.1]; "capabilities" means the turtle toggle, completion notifications, the magnet confirm and a details view [OR §4], which §6.1 makes T1 and §4.8 adapts (staged add instead of a confirm popup). |
```

```diff
-- Clients see the socket close, show "reconnecting", and retry after 0.5, 1, 2, 4 and 8 s (then
-  every 10 s). A successful reconnect brings a new `instance` id and a fresh snapshot.
+- Clients see the socket close, show "reconnecting", and retry after 0.5, 1, 2, 4 and 8 s (then
+  every 10 s). A successful reconnect brings a new `instance` id and a fresh snapshot. Why these
+  numbers: a systemd restart after a crash takes `RestartSec=2` plus the time to `READY=1`, so
+  the 2 s and 4 s retries catch it within one or two attempts without hammering a daemon that is
+  crash-looping (the unit allows 20 starts per minute, §3.2.2); the 10 s cap is the old widget's
+  idle poll (`manifest.json:19`), so the bar is never staler than it is today; and the same
+  ladder is §4.4 rule 4, so QML and Rust clients behave alike in tests.
```

```diff
 - **Mojibake repair.** If a name round-trips as UTF-8 that was decoded as Latin-1
   ("è¾›ç‰¹å°”"), re-decode it; otherwise leave it unchanged. A unit test covers the captured sample
-  from `screens/07-search-sintel-16s.txt`.
+  from `screens/07-search-sintel-16s.txt`. Why this test and no other: re-encoding the string as
+  Latin-1 and decoding the bytes as UTF-8 succeeds only when every byte sequence is a valid
+  multibyte UTF-8 character, which a real Latin-1 title ("Amélie", "Straße") almost never is, so
+  the repair cannot damage a name that was right; and it is reversible, so a false positive is
+  recoverable from the provider's raw bytes, which the daemon keeps for the session.
```

```diff
 The tab set follows qBittorrent's property tabs (General, Trackers, Peers, Content, Speed;
 `qBittorrent src/gui/properties/proptabbar.cpp:50-100`) minus HTTP sources (§2.6) plus Pieces
-and Log; the "Speed" tab's graph becomes the status-line sparkline (P6-03).
+and Log; the "Speed" tab's graph becomes the status-line sparkline (P6-03). Why the two
+additions: Pieces is the one view that shows sequential plus first/last working, which is what
+a streaming user doubts first when a file will not play (§6.2 row 25); Log answers "why is this
+torrent not moving" next to the torrent, where today the message disappears after 8 or 15 s
+([RC §9] #13).
```

```diff
-| Private flag (P6-12) | Verify the fork skips DHT and PeX for torrents with `private=1`; fix if not | none | none | none | n/a | A private fixture torrent with a local tracker: no DHT queries in the engine log |
+| Private flag (P6-12) | Verify the fork skips DHT and PeX for torrents with `private=1`; fix if not. Why verify: upstream threads `info.private` into the session (`vendor/rqbit/librqbit/src/session.rs:1115,1125,1365-1386`, `is_private`), but the fork replaced the runtime and the socket layer, and no app test covers the flag; a leak would announce a private tracker's torrent on the public DHT | none | none | none | n/a | A private fixture torrent with a local tracker: no DHT queries in the engine log |
```

```diff
 - `robot-docs` and `capabilities` move to contract 1.2.0 (P1-11): the 32 KiB/s cap, the daemon,
   `serve`, `restart`, `dashboard`, `log`, `events`, `api`, `--no-start`, the "reads never start,
-  writes do" rule, the new `status` shape, `--until playable` and a pointer to `docs/IPC.md`.
+  writes do" rule, the new `status` shape, `--until playable` and a pointer to `docs/IPC.md`.
+  Why a minor bump: every change is additive for an agent that follows `robot-docs` (new verbs,
+  new optional fields) except the `status` stopped shape, which is declared under
+  `capabilities.changes[]` with `breaking: true` (Appendix F, Q12); a 2.0.0 would make agents
+  re-read a contract that is otherwise unchanged.
```

---

## Section D. Grounding

### D.1 Citations: 40 sampled, evenly spaced over the 321 unique `path:line` citations

Method: `r3val/cites.py` extracted 389 citations (321 unique after de-duplication); `citedump.py`
took every 8th and printed the cited lines from the resolved file; three that my resolver could
not map (`~/.config/...`, `README.md`, `Fragments utils.rs`) I checked by hand. Status: **40/40
correct**.

| # | PLAN line | § | Citation | Status |
|---|---|---|---|---|
| 1 | 46 | 0.1 | `src/main.rs:786-827` | OK (`run_native_app`, `backend::open_app` at 787) |
| 2 | 116 | 0.3 | `shell.qml:1161-1172` | OK (`isBarWidgetPanelPlugin` excludes panel/overlay/menu kinds) |
| 3 | 170 | 0.4 | `vendor/rqbit/librqbit/src/file_info.rs:14-25` | OK (`iter_piece_priorities`, first/last) |
| 4 | 261 | 1.2 | `src/tui/mod.rs:1670-1674` | OK (Every 2000 ms Poll, 300 ms Hover) |
| 5 | 275 | 1.2 | `src/engine.rs:588` | OK (`app/version`) |
| 6 | 284 | 1.2 | `src/agent.rs:134-210` | OK (`capabilities_json`) |
| 7 | 293 | 1.3 | `src/engine.rs:517-546` | OK (`Drop for AppEngine`, 4 s bound) |
| 8 | 297 | 1.3 | `src/tui/mod.rs:2199` | OK ("seeds, leechers") |
| 9 | 300 | 1.3 | `src/tui/mod.rs:1178` | OK (Ctrl+C `request_close` + quit) |
| 10 | 309 | 1.3 | `~/.config/omarchy/extensions/omarchy-menu.jsonc:33-36` | OK (stop gated on `qbittorrent-nox` at 36) |
| 11 | 360 | 1.7 | `src/network_app.rs:78-90` | OK (`controls` route match) |
| 12 | 478 | 2.3 | `README.md:79-86` | OK (ProtonVPN binding paragraph) |
| 13 | 632 | 3.2.2 | `src/search.rs:945-961` | OK (`shutdown_all`) |
| 14 | 716 | 3.2.5 | `src/ipc.rs:136-164` | OK (`InstanceLock::acquire_in`) |
| 15 | 817 | 3.2.8 | `src/tui/mod.rs:2148` | OK (`network_app::state` per render) |
| 16 | 1119 | 3.4.7 | `torrent_state/mod.rs:212` | OK (`pub(crate) state_change_notify: Notify`) |
| 17 | 1218 | 3.4.10 | `src/ipc.rs:96-100` | OK (95 s add timeout) |
| 18 | 1296 | 3.7 | `src/rss.rs:69-71,93-100` | OK (path; lock file 0600) |
| 19 | 1347 | 3.8 | `src/main.rs:1296-1300` | OK (`notify` with `-i qbittorrent`) |
| 20 | 1454 | 4.1 | `plugin/oliver.torrents/Panel.qml:124-139` | OK (`IpcHandler`, `cursor()`) |
| 21 | 1474 | 4.2.1 | `Style.qml:329-336` | OK (type scale 10..28) |
| 22 | 1482 | 4.2.1 | `Style.qml:33,464-471` | OK (`cornerRadius`, hyprctl rounding) |
| 23 | 1510 | 4.2.1 | `Style.qml:40-44` | OK (hover-cursor comment) |
| 24 | 1544 | 4.2.3 | `PanelHero.qml:95-104` | OK (uppercase caption, bold, letterSpacing 1.2, dim) |
| 25 | 1574 | 4.2.4 | `Panel.qml:433` | OK (`󰏤` paused glyph) |
| 26 | 1583 | 4.2.4 | `Panel.qml:454` | OK (`hoverColor: root.urgent`) |
| 27 | 1654 | 4.3.2 | `omaqbt TorrentTable.qml:225-226` | OK (`!` in urgent on error) |
| 28 | 1804 | 4.5 | `Panel.qml:149` | OK (right click adds; 150 middle opens TUI) |
| 29 | 1877 | 4.6 | `Ui/ConfirmDialog.qml:13-15,52-58` | OK (scrim alpha 0.7; width 370) |
| 30 | 2001 | 4.9 | `Ui/ConfirmDialog.qml:122` | OK (`onEntered: root.selectedIndex = index`) |
| 31 | 2072 | 4.13 | `Style.qml:360-367` | OK (`reduceMotion`, `duration`) |
| 32 | 2175 | 4.18 | `Fragments utils.rs:127-139` | OK (`eta_text`: Paused/Queued/Checking…; file is `src/utils.rs`) |
| 33 | 2346 | 5.5.5 | `src/tui/mod.rs:2229` | OK (`overlay` check) |
| 34 | 2512 | 6.2 | `vendor/rqbit/librqbit/src/session.rs:140` | OK (`pub ratelimits: Limits`) |
| 35 | 2534 | 6.2 | `session.rs:569-570` | OK (UPnP refused) |
| 36 | 2578 | 6.3 | `chunk_tracker.rs:205` | OK (`get_have_pieces`) |
| 37 | 2628 | 7.2 | `src/browse.rs:60` | OK (16 MiB cap) |
| 38 | 2929 | P0-07 | `plugin/oliver.torrents/Service.qml:78` | OK (`reconcile()`) |
| 39 | 3197 | P1-08 | `torrent_state/mod.rs:212` | OK (second citation site) |
| 40 | 4605 | A.10 | `src/engine.rs:1397-1404` | OK (progress from `progress_bytes/total_bytes`) |

Second sweep (not required, run because lines move): 180 further citations across §0-§9 and the
appendices checked by first and last line (`r3val/sweep2.txt`). All resolve to the claimed
content except these five, each off by a line or swapped:

| PLAN line | Citation as written | Correct | Note |
|---|---|---|---|
| 1185, 3221 | `src/main.rs:550-556` | `src/main.rs:550-560` | the `before` set is at 550; the diff that finds the new hash is 557-560 |
| 1481 | `Commons/Border.qml:141,217,17` for `controlSpec`, `surfaceSpec`, `flat` | `Commons/Border.qml:217,141,17` | `surfaceSpec` is at 141 and `controlSpec` at 217 (swapped) |
| 1479 | `plugins/bar/Bar.qml:86-91` for `foreground`, `urgent`, `fontFamily`, `foregroundAnimationEnabled` | `plugins/bar/Bar.qml:78,84-91` | `fontFamily` is 78, `foreground` 84 |
| 1484 | `Ui/PanelSectionHeader.qml:20` | `Ui/PanelSectionHeader.qml:19` | the `Qt.darker(foreground, 1.4)` line is 19 |
| 1730 | `services/PluginShellApi.qml:29-31` | `services/PluginShellApi.qml:30-32` | `serviceFor` is 30-32; 29 is blank |
| 293, 2949 | `ftui-runtime-0.6.0/src/program.rs:111-117` "the runtime clears it" | `:110-117` are the wrappers; the loop polls at `:5254-5441`, the clear is `:5490` in `complete_lifecycle` after `shutdown_once` (`:5484`) | misleading rather than wrong; see A.3 |

Hunks:

```diff
-  concurrent adds (`src/main.rs:525-532`, `550-556`) [RC §7].
+  concurrent adds (`src/main.rs:525-532`, `550-560`) [RC §7].
```

```diff
-| Borders | `Border.controlSpec(state, fg, accent)`, `Border.surfaceSpec("popups","border",…)`, `Border.flat(color, w)` | theme | `Commons/Border.qml:141,217,17` |
+| Borders | `Border.controlSpec(state, fg, accent)`, `Border.surfaceSpec("popups","border",…)`, `Border.flat(color, w)` | theme | `Commons/Border.qml:217,141,17` |
```

```diff
-| Bar | `bar.foreground`, `bar.urgent`, `bar.fontFamily`, `bar.foregroundAnimationEnabled` | theme | `plugins/bar/Bar.qml:86-91`; `Ui/Panel.qml:23` |
+| Bar | `bar.foreground`, `bar.urgent`, `bar.fontFamily`, `bar.foregroundAnimationEnabled` | theme | `plugins/bar/Bar.qml:78,84-91`; `Ui/Panel.qml:23` |
```

```diff
-| Dim text | `Model.dim(foreground, background)`: the first of `Qt.darker(foreground, 1.4)` (the shell's convention), `Color.muted`, and a blend of `foreground` toward `background` at 0.75 then 0.85 that reaches 4.5:1; `foreground` itself as the last resort | theme | `Ui/PanelHero.qml:23`, `Ui/PanelSectionHeader.qml:20`; the same ladder as the TUI's §5.5.1, one fixture (`contracts/ui/themes/`) |
+| Dim text | `Model.dim(foreground, background)`: the first of `Qt.darker(foreground, 1.4)` (the shell's convention), `Color.muted`, and a blend of `foreground` toward `background` at 0.75 then 0.85 that reaches 4.5:1; `foreground` itself as the last resort | theme | `Ui/PanelHero.qml:23`, `Ui/PanelSectionHeader.qml:19`; the same ladder as the TUI's §5.5.1, one fixture (`contracts/ui/themes/`) |
```

```diff
-`service`/`serviceFor` facade (`services/PluginShellApi.qml:29-31`) [FO §2.2].
+`service`/`serviceFor` facade (`services/PluginShellApi.qml:30-32`) [FO §2.2].
```

```diff
-| D2 | Closing the terminal window kills the process without running `Drop`, leaving a stale `engine.sock`. | Only SIGPIPE is handled by the app (`src/main.rs:288`); ftui's own signal thread force-exits with `128+sig` when the runtime does not acknowledge within `SIGNAL_SHUTDOWN_GRACE = 2 s` (`ftui-core-0.6.0/src/terminal_session.rs:107,147-167,1270-1295`), which matches the observed 2.2 s (`evidence/app-lifecycle-observations.txt`); `Drop` at `src/engine.rs:517-546`, `src/ipc.rs:278-286` never runs | P0-08, then P1-03 |
+| D2 | Closing the terminal window kills the process without running `Drop`, leaving a stale `engine.sock`. | Only SIGPIPE is handled by the app (`src/main.rs:288`); ftui's own signal thread force-exits with `128+sig` when the runtime does not acknowledge within `SIGNAL_SHUTDOWN_GRACE = 2 s` (`ftui-core-0.6.0/src/terminal_session.rs:107,147-167,1270-1295`); the runtime's loop polls the signal (`ftui-runtime-0.6.0/src/program.rs:5254-5441`) but clears it only after `shutdown_once` in `complete_lifecycle` (`:5484-5490`), which matches the observed 2.2 s (`evidence/app-lifecycle-observations.txt`); `Drop` at `src/engine.rs:517-546`, `src/ipc.rs:278-286` never runs | P0-08, then P1-03 |
```

(`src/main.rs:550-556` also appears at L3221 inside P1-09; the A.5 hunk already changes that
line.)

### D.2 Numeric claims recomputed (`r3val` Python, from the installed `colors.toml` files)

| # | Claim (where) | Recomputed | Status |
|---|---|---|---|
| 1 | Fixed columns 26+110+76+120+80+80+64+56+56+80 = 748; without ADDED/RATIO/↑ = 532; with ↑ kept = 612 (§4.3.2, §4.3.3) | 748 / 532 / 612 | OK |
| 2 | Chrome 636 / 246 / 16; details undock cost 360+14+16 = 390 (§4.2.2, §4.3.3) | 636 / 246 / 16 / 390 | OK |
| 3 | NAME per tier: ≥1600 → ≥216; 1400-1599 → 232-431; 1100-1399 → 322-621; 900-1099 → 352-551; with ↑ at 1400 → 152-351 (§4.3.3) | 216; 232-431; 322-621; 352-551; 152-351 | OK |
| 4 | 512-frame queue: 8 topics × 60 s ≈ 480 (§3.4.7) | 480 | OK |
| 5 | `Pieces.have`: 100,000 pieces → 16.7 KB base64; 200,000 under 64 KiB (A.9, P6-02) | 12,500 B → 16,668 B; 25,000 B → 33,336 B | OK |
| 6 | 1 MiB holds a 256-item chunk at < 2 KiB each (§3.4.1); 32 MiB ≈ 30 s of 1080p (§3.5) | 512 KiB; 33.5 s at 8 Mbps | OK |
| 7 | Owner's theme: `muted #7a7a7a` ≈ 4.9:1, `dark_foreground #505050` ≈ 2.6:1 (D16); `Qt.darker(fg,1.4)` = `#b6b6b6` "about 10.6:1" (§4.2.1) | 4.9; 2.6; **10.4** | OK except 10.6 → 10.4 (hunk below) |
| 8 | Dim 1.4 on `catppuccin-latte` 10.2, `everforest` 3.8, `gruvbox` 4.3, `tokyo-night` 4.3; `rose-pine` fg 6.7, every other theme > 7:1; `muted` ≥ 4.5 on 3 of 23 themes (§4.2.1, §5.5.1) | 10.2 / 3.8 / 4.3 / 4.3; 6.7; min of others 7.1; `ethereal`, `starship`, `vantablack` = 3 of 23 | OK |
| 9 | Accent/background: `catppuccin-latte` 4.3, `miasma` 3.9, `rose-pine` 3.1 fail 4.5 (§4.2.1, §5.5.1) | 4.3 / 3.9 / 3.1 | OK |
| 10 | **Accent-vs-foreground fallback "the owner's `#8d8d8d` against `#ffffff` qualifies"** (§5.5.1), and P4-05's Accept "the accent fallback triggers on the owner's theme" | channel difference **114** (rule says < 32), luminance ratio **3.32** (rule says ≤ 1.3); accent/bg 6.3 (passes 4.5) | **FAILS**: by the rule as written the owner's theme does not trigger either clause. The rule triggers on `kanagawa` (accent equals foreground: difference 0, ratio 1.00) and, via the 4.5:1 clause, on `catppuccin-latte`, `miasma`, `rose-pine`; `tokyo-night` (47, 1.19) correctly does not |
| 11 | Low-chroma: `#a4a4a4` vs `#b6b6b6` differ by 18 (§5.5.2); triggers on the owner's theme and not on `tokyo-night` (P4-05) | 18; tokyo-night red/green difference 89 (also triggers on `hackerman` 29, `lumon` 17, `white` 16, which the plan does not claim either way) | OK |
| 12 | STATE 110 = 11 cells of 13 px subtitle (≈ 86 px) + 2 × 10; NAME 210 ≈ 26 cells (§4.3.2) | 11 × 7.8 = 85.8 + 20 = 105.8 ≤ 110; 210 / 7.8 = 26.9 | OK |
| 13 | `vendor/rqbit` 3.3 MB, `vendor/asupersync` 57 MB, `vendor/nova3` 64 KB, `vendor/` 60 MB, `docs/validation` 279 MB and 2,197 files in six dated directories; `src/tui/mod.rs` 3,504 lines, `src/engine.rs` 2,694; installed binary 21,555,840 bytes; 19 modified files (§1.1, §1.7) | 3.3M / 57M / 64K / 60M / 279M, 2,197, 6; 3,504 / 2,694; 21,555,840; 19 | OK |
| 14 | Critical path weights 35/37/44/40/44/44 and "17 items deep" (§9.4) | 35/37/44/40/44/44; 17 nodes = 15 items + 2 milestones | weights OK; count wording fixed in B.2 |

Hunks for 7 and 10:

```diff
-  on black (about 10.6:1) and on `catppuccin-latte` 10.2:1, but on `everforest` it is 3.8:1 and on
+  on black (about 10.4:1) and on `catppuccin-latte` 10.2:1, but on `everforest` it is 3.8:1 and on
```

```diff
-| accent (focused border, keys, sorted header) | `accent` | if `accent` and `foreground` differ by less than 32 in every RGB channel **and** are within 1.3:1 of each other (the owner's `#8d8d8d` against `#ffffff` qualifies by luminance; tokyo-night's blue accent on a grey-white foreground, luminance ratio 1.19, does not, because hue carries the difference), keys use bold `foreground` instead of accent; and whenever `accent` is below 4.5:1 against `background` (`catppuccin-latte` 4.3, `miasma` 3.9, `rose-pine` 3.1), keys and the ready badge use bold `foreground`. A luminance-only rule ("within 2.0:1") would strip accent from most dark themes and make the TUI look less like the shell's own panels |
+| accent (focused border, keys, sorted header) | `accent` | if `accent` and `foreground` differ by less than 32 in every RGB channel **and** are within 1.3:1 of each other (`kanagawa`, whose accent equals its foreground, qualifies; the owner's `#8d8d8d` against `#ffffff` does not: the channels differ by 114 and the luminance ratio is 3.3, so the grey accent stays distinguishable and is used; tokyo-night's blue accent on a grey-white foreground, luminance ratio 1.19 but channel difference 47, does not either, because hue carries the difference), keys use bold `foreground` instead of accent; and whenever `accent` is below 4.5:1 against `background` (`catppuccin-latte` 4.3, `miasma` 3.9, `rose-pine` 3.1), keys and the ready badge use bold `foreground`. A luminance-only rule ("within 2.0:1") would strip accent from most dark themes and make the TUI look less like the shell's own panels |
```

```diff
-  low-chroma fallback triggers on the owner's theme and not on `tokyo-night`; the accent fallback
-  (§5.5.1) triggers on the owner's theme and on `catppuccin-latte`, `miasma`, `rose-pine`, and
-  not on `tokyo-night`; the dim hex per fixture theme equals `contracts/ui/themes/expected.json`.
+  low-chroma fallback triggers on the owner's theme and not on `tokyo-night`; the accent fallback
+  (§5.5.1) triggers on `catppuccin-latte`, `miasma`, `rose-pine` (below 4.5:1 on their
+  background) and `kanagawa` (accent equals foreground), and not on the owner's theme (6.3:1,
+  channel difference 114) or `tokyo-night`; the dim hex per fixture theme equals
+  `contracts/ui/themes/expected.json`.
```

### D.3 Named platform APIs: every one exists in the installed or vendored source

| Group | API named in the plan | Found at |
|---|---|---|
| ftui 0.6 | `table.rs`, `tabs.rs`, `sparkline.rs`, `virtualized.rs`, `log_viewer.rs`, `toast.rs`, `notification_queue.rs`, `badge.rs`, `scrollbar.rs`, `command_palette/` | `ftui-widgets-0.6.0/src/` (all present) |
| ftui 0.6 | `charts.rs`, `image.rs`, `canvas.rs` | `ftui-extras-0.6.0/src/` |
| ftui 0.6 | `TableState::handle_mouse`; the widget sorts by cell text (`sort_column`) | `ftui-widgets-0.6.0/src/table.rs:578`; `:200,228-259` |
| ftui 0.6 | `Subscription`, `Every` | `ftui-runtime-0.6.0/src/subscription.rs:33,477` |
| ftui 0.6 | `BorderType::Rounded` | `ftui-widgets-0.6.0/src/block.rs:598` |
| ftui 0.6 | `ftui_core::shutdown_signal::{pending_termination_signal, clear_pending_termination_signal}`, `with_test_signal_serialization`, `SIGNAL_SHUTDOWN_GRACE` | `ftui-core-0.6.0/src/lib.rs:79-127`; `terminal_session.rs:107` |
| Quickshell | `Socket`, `SplitParser`, `Process` | `/usr/lib/qt6/qml/Quickshell/Io/quickshell-io.qmltypes:197,285,50` |
| Quickshell | `IpcHandler` | not in `Io/`; it is the `Quickshell.Io` import the plugin already uses (`plugin/oliver.torrents/Panel.qml:124`); the qmltypes name is not `IpcHandler` in `quickshell-io.qmltypes` (declared in `quickshell-core.qmltypes` family); the plugin compiles today, so it exists |
| Quickshell | `FloatingWindow`, `minimumSize`, `implicitWidth`/`Height` | `_Window/quickshell-window.qmltypes:40` (`FloatingWindowInterface`, exported as `Quickshell._Window/FloatingWindow`), `:57`, `:303` |
| Quickshell | `Quickshell.env`, `Quickshell.clipboardText`, `execDetached`, `ScriptModel` | `quickshell-core.qmltypes:1055,998,1061,1431` |
| Quickshell | `Socket.write` adds no newline, `flush()`; `SplitParser` buffers unbounded | `quickshell-src/src/io/socket.hpp:37-43`; `datastream.cpp:37-41,91-98` |
| Omarchy shell Ui | `Panel`, `KeyboardPanel`, `PanelKeyCatcher`, `PanelHero`, `PanelSectionHeader`, `PanelSeparator`, `PanelActionButton`, `PanelToolTip`, `CursorSurface`, `ConfirmDialog`, `FilterInput`, `BorderSurface`, `ToggleSwitch`, `Dropdown`, `Button`, `TextField`, `BarIconButton` | `/usr/share/omarchy/shell/Ui/*.qml` (all present) |
| Omarchy shell Ui | `PanelKeyCatcher.returnRequested`, `activateRequested`, `textKey`, `blocked`, `reorderable` | `Ui/PanelKeyCatcher.qml:44,43,49,36,39` |
| Omarchy shell Ui | `ConfirmDialog.selectedIndex: 1`, `confirmed()`, `handleKey` | `Ui/ConfirmDialog.qml:12,22,24` |
| Omarchy Commons | `Style.space`, `Style.spacing.{xxs…huge, controlGap, controlPaddingX, controlPaddingY, controlHeight, popupRowHeight, rowGap, rowPaddingX, labelGap, panelGap, panelPadding, popupPadding}`, `Style.font.{caption…displayLarge, iconSmall, icon, iconLarge}`, `Style.reduceMotion`, `Style.duration`, `Style.cornerRadius`, `Style.{normalFill, hoverFill, selectedFill, pressedFill, selectionFill, selectedAccentFill}` | `Commons/Style.qml` (every one found by name) |
| Omarchy Commons | `Color.{foreground, background, accent, urgent, muted}`, `Color.popups`, `Color.tooltip`, `Color.menu`; `muted` falls back to `color8` | `Commons/Color.qml:19-23,84,89,100,168` |
| Omarchy Commons | `Border.controlSpec`, `Border.surfaceSpec`, `Border.flat`; `Util.alpha`, `Util.execArgv` | `Commons/Border.qml:217,141,17`; `Commons/Util.qml:44,77` |
| Omarchy services | `shell.hide`, `serviceFor`, `PluginRegistry.isEnabled`, `entryPointUrl`, `computePanelEntries`, `omarchy-shell shell summon` | `services/PluginShellApi.qml:50,30`; `services/PluginRegistry.qml:148,118`; `shell.qml:1356`; `/usr/share/omarchy/bin/omarchy-shell:58` |
| Omarchy bar | `bar.foreground`, `bar.urgent`, `bar.fontFamily`, `bar.foregroundAnimationEnabled`; the 380 popup width in `network`, `bluetooth`, `tailscale`, `agents` | `plugins/bar/Bar.qml:84,89,78,86`; `plugins/panels/{network:1109,bluetooth:670,tailscale:421}/Panel.qml`, `plugins/agents/Panel.qml:972` |
| shell-dev guide | "Keeping state current without polling", "Plugin contract", "Palette references" | `qsr/omarchy-src/agents/skills/shell-dev.md` |
| librqbit fork | `state_change_notify` (+ `notify_waiters` at 333, 412-460, 518-525), `with_chunk_tracker`, `ChunkTracker::get_have_pieces`, `per_peer_stats_snapshot`, `PeerStats`, `From<&Peer>`, `Peer.bitfield: BF`, `has_full_torrent`, `Session.ratelimits: Limits`, `set_download_bps`, `set_upload_bps`, live `ratelimits: Limits`, `trackers`, `concurrent_init_limit`, `create_torrent_file.rs`, `iter_piece_priorities`, fastresume refusal, `version()` | `torrent_state/mod.rs:212,266,186`; `chunk_tracker.rs:205`; `torrent_state/live/mod.rs:782,217`; `live/peer/stats/snapshot.rs:27,57`; `live/peer/mod.rs:259,297`; `session.rs:140,567-570`; `limits.rs:89,85`; `src/engine.rs:432`; `initializing.rs:113-114`; `lib.rs:97` |
| nova3 | pool of `min(cpu_count, 4)` | `vendor/nova3/nova2.py:71-75,262-264` |
| Host tools | `notify-send -A`, `systemd-inhibit`, `grim`, `wl-paste`, `wl-copy`, `wtype`, `hyprctl`, `omarchy-shell`, `omarchy-theme-set`, `omarchy-launch-or-focus-tui` | all on `PATH` (`which`); `notify-send --help` lists `--action` |

Not found: nothing. One naming note: `FloatingWindow` is exported by the type
`FloatingWindowInterface`; the plan's `quickshell-window.qmltypes:47-104` range covers `title` to
`parentWindow` and includes `minimumSize` (57) but `implicitWidth` is at 303 (inherited); harmless.

---

## Section E. Owner questions (only the genuinely theirs)

1. **Shows: Enter stages or downloads?** W2 step 3 says "Enter downloads a release"; the dashboard's
   Search and RSS views stage (P3-12, P3-14, §4.8). The A.6 hunk makes the dashboard's Shows view
   stage too and keeps the TUI direct. Confirm, or make Shows the one direct-download view in the
   dashboard (then P3-13 does not need P3-06/P3-11).
2. **Accent on the monochrome theme.** Under the §5.5.1 rule as written your `#8d8d8d` accent is
   used as is (6.3:1 on black, clearly darker than white text). The plan's prose assumed the
   fallback would strip it to bold white. Which do you want on your theme: grey accent for keys
   and the ready badge, or bold white? (Section D.2 #10; the hunks assume grey, as the rule says.)
3. **Rename revert.** The A.1 hunk adds `Record.display_name` so a rename can be cleared back to the
   torrent's own name. If you never want the original name back, `Record.name` alone is simpler.
4. **Bulk remove keeps files only.** The A.8 hunk makes `X` keep-files only and leaves "delete files"
   per torrent. If you want a bulk delete-files, it needs its own confirm text and no undo.
5. **Undo scope.** Per batch (`undo` with no hash restores the last removal, one or many) as hunked,
   or per hash only?

---

## Verification of the hunks against `PLAN.md`

Every `-` line and every context line (leading space) in the ```diff blocks above was checked for
verbatim presence in `docs/plan-2026-10-18/PLAN.md` by `r3val/verify_hunks.py`. Output:

```
diff blocks: 76; removed lines: 131; context lines: 30; added lines: 292
every removed and context line exists verbatim in PLAN.md
removed lines that occur more than once in PLAN.md (the hunk needs its neighbours to disambiguate): 3
  report L218 x8: - *Depends on:* P0-04.
  report L753 x3: - *Depends on:* P4-01.
  report L771 x2: - *Depends on:* P1-06.
(The P4-01 and P1-06 hunks carry the unique line that precedes the Depends line as context, which fixes the anchor; `- *Depends on:* P0-04.` sits inside the P0-08 hunk between that item's unique Tests lines, so it is unambiguous there; the other P0 occurrences are addressed by the prose note in B.3.)
```
