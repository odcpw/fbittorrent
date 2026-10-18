# Round 2 review: architecture, daemon, API, engine, migration, security, hygiene, tests, DAG

Reviewer lens: architecture and engineering only. Product and look-and-feel are a sibling review.
Read completely: `PLAN.md` (4,676 lines), `review-log.md`, `README.md`, `reality-check.md`,
`frontend-options.md` §2.6/§4.2/§7, `omatorrent-research.md` §2.1/§6/§8, the evidence files, and
the cited code: `src/main.rs`, `src/ipc.rs`, `src/qbt.rs`, `src/network_app.rs`, `src/backend.rs`,
`src/engine.rs`, `src/native_runtime.rs`, `src/agent.rs`, `src/search.rs`, `src/config.rs`,
`src/tui/mod.rs` (structure and every cited line), `plugin/oliver.torrents/*`, `Cargo.toml`,
`vendor/rqbit/README.md`, `vendor/rqbit/librqbit/src/{session,limits,file_info,chunk_tracker}.rs`,
`torrent_state/{mod,initializing}.rs`, `live/{mod,peer/mod,peer/stats/snapshot}.rs`,
`vendor/nova3/nova2.py`, `vendor/asupersync/{FBITTORRENT_PATCH.md,src/runtime/builder.rs}`,
`docs/engine-evaluation.md`, `docs/queue-encryption-vpn-plan.md`, the Quickshell sources
`src/io/{socket,datastream}.{hpp,cpp}`, Sprout's `docs/IPC.md` and `internal/ipc/server.go`, the
installed Omarchy shell files the plan cites, and `systemctl --user` defaults on this machine.
Nothing under `src/`, `plugin/`, `vendor/`, `Cargo.*` or `PLAN.md` was modified.

Diff hunks below quote `PLAN.md` lines verbatim (`-`) and give the replacement (`+`); the `@@`
header names the first quoted line.

## Verdict

The plan is implementable by a fresh agent for about 90 % of its surface: the daemon decision,
the NDJSON transport, the subscription model, the `caps` negotiation and the item graph are
consistent, grounded and acyclic (Section D). It is not implementable as written in a handful of
places where a fresh agent would either follow an instruction that cannot work (`cp` over a running
binary in the M1 demo; `native.py` with the *unchanged* v0.1 TUI as a client of the daemon;
`events --since-seq` when no event carries a sequence number; a `Pieces` frame that can exceed the
protocol's own line cap) or has to guess at a lifetime edge the plan leaves open (what the CLI does
when the daemon drains under its request; which processes systemd kills first; what the existing
`autostart` key means after P1-04). The agent/CLI contract survives the split except for two exit
codes that contradict the dictionary they claim to keep (`busy` and `status --quiet` mapped to 1).

Top five risks, in order:

1. **Lifetime edges are under-specified where races live.** Idle exit, `bye`, `systemctl start`
   returning on an already-active-but-exiting unit, `KillMode=control-group` tearing down Python
   provider pools and `systemd-inhibit` before the daemon's own drain, and `StartLimitBurst=5`
   under an on-demand unit that starts often (A9, A10). The state machine of §3.2.7 is right; the
   transitions a client observes mid-request are not written down.
2. **Cold-start latency is hidden by a sentence.** §3.2.4 makes `torrents.add` wait for `Running`,
   and `Running` today means every record restored and checked in series (`src/engine.rs:453-466`,
   `concurrent_init_limit: Some(1)`). Under A1 the daemon restarts after every idle period, so the
   first magnet click after five idle minutes pays the full check of every *incomplete* torrent even
   after P1-16 (B4).
3. **The agent surface has two holes that break the "contract intact" promise** (`--since-seq`
   without a `seq`; `busy` → exit 1 "retrying rarely helps" while `retryable: true`) and one
   omission (`register-magnet` is a real command absent from Appendix F) (A3, A7, A8).
4. **P1 is four L items in series** (P1-02, P1-03, P1-06, P1-08) before the first user-visible
   change; the weighted critical path to M1 is 35 S-units (Section D). P1-02 bundles three
   refactors whose isomorphism test only makes sense for the first (B3).
5. **Landing hygiene has one incoherent commit** (C3 edits `Cargo.toml` for a crate the legacy
   manifest does not depend on) and the git-fork decision adds a network dependency to every
   `--locked` build; both are cheap to fix now and expensive after the first push (A13).

Grounding: 44 `file:line` citations were spot-checked; 38 are exact, 6 are wrong or imprecise
(A5). The DAG is acyclic, §9.3 and §9.4 agree on every edge, one topological level is off (M6).

---

## A. Defects (must fix)

### A1. The M1 demo tells the operator to `cp` over a running binary, which fails with `ETXTBSY`

- **Location:** §9.7 M1 demo step 9, line 3876.
- **What is wrong:** Linux refuses to open a running executable for writing; `cp` onto
  `~/.local/bin/fbittorrent` while the daemon runs prints "Text file busy". §8.6 (line 2579-2581)
  already prescribes copy-to-temp-then-rename for exactly this reason, and §3.2.12's replacement
  check compares `(dev, inode, mtime)` of the *path*, which only changes under rename. The demo
  step contradicts both.
- **Evidence:** `PLAN.md:2579-2581` (install.sh rule); POSIX `ETXTBSY` semantics.
- **Fix:**

```diff
@@ PLAN.md:3876 @@
-9. `cp` a rebuilt binary over `~/.local/bin/fbittorrent` while a download runs → the popup shows
-   "Update installed; restarts when idle".
+9. Install a rebuilt binary with `scripts/install.sh` (copy to `fbittorrent.tmp`, then `mv` over
+   the old file; a plain `cp` fails with "Text file busy" while the daemon runs) while a download
+   runs → within 30 s the popup shows "Update installed; restarts when idle"; `hello` still
+   reports the old `server.build`.
```

### A2. P1-10's acceptance cannot be met: the unchanged v0.1 TUI can never be a client of the daemon

- **Location:** P1-10 (lines 3013-3021) and §3.6 (lines 1210-1214).
- **What is wrong:** P1-10 says "`native.py` passes with the unchanged TUI running as a client of
  the daemon". The v0.1 TUI does not have a client mode: `run_native_app` first checks
  `ipc::is_running` and, if a socket answers, calls `backend::open_app()` (launch-or-focus a
  terminal) instead of attaching; otherwise it acquires `engine.lock` and starts its own engine
  (`src/main.rs:786-809`). With the daemon holding the lock and serving `engine.sock`, the old TUI
  either focuses a window or fails "already open". The only v0.1 clients the compat adapter can
  serve are the v0.1 CLI and the v0.1 widget (`status --json`, `start`, `stop`, `delete`, …). The
  TUI becomes a client in P1-15, which depends on P1-10.
- **Evidence:** `src/main.rs:787` (`if ipc::is_running(&cfg) { return backend::open_app(); }`),
  `src/main.rs:804-806` (`engine::start` → "already open" → `open_app()`), `src/ipc.rs:147-157`
  (the lock), P1-15 Depends `P1-10`.
- **Fix:**

```diff
@@ PLAN.md:3013 @@
 **P1-10 Legacy socket adapter** (S)
 - *Do:* The daemon serves `engine.sock` with the 4-byte framing; requests parse into `EngineOp`;
   responses use the qBittorrent-shaped JSON; `app/shutdown` stops the daemon and `app/restart`
   restarts the engine.
-- *Why:* the v0.1 TUI and widget keep working during the migration (§3.6).
-- *Accept:* `native.py` passes with the unchanged TUI running as a client of the daemon.
-- *Tests:* `native.py --daemon` (a mode that starts the daemon first and runs the same 11 cases);
-  unit `legacy::tests::route_parse_roundtrip`.
+- *Why:* the v0.1 CLI and the v0.1 bar widget keep working during the migration (§3.6). The v0.1
+  TUI cannot attach to any engine but its own (`src/main.rs:787,804-806`); it becomes a client in
+  P1-15.
+- *Accept:* every CLI call `native.py` makes today (`status`, `list`, `add`, `stop`, `start`,
+  `delete`, `rss …`, `capabilities`) answers identically from a v0.1 binary pointed at the
+  daemon's `engine.sock`; `plugin/oliver.torrents/Service.qml` (v0.1, `status --json` polling)
+  renders the daemon's torrents. A v0.1 `fbittorrent tui` is out of scope: it sees a live
+  `engine.sock` and runs `backend::open_app()` (`src/main.rs:787`), and nothing the daemon can
+  answer changes that; `install.sh` replaces the binary, so no v0.1 TUI remains after the upgrade.
+- *Tests:* `native.py --legacy-cli <v0.1 binary> --daemon <v0.2 binary>` (runs the CLI steps of the
+  11 cases against the daemon with the pinned v0.1.2 binary from the `M0` tag); unit
+  `legacy::tests::route_parse_roundtrip`; the TUI-as-client cases move to P1-15.
```

```diff
@@ PLAN.md:1210 @@
-A new CLI that finds only an old, TUI-hosted `engine.sock` (no `api.sock`) still reads through it
-but refuses writes with "an older fbittorrent app is running; quit it with Q and retry". Why: mixing
-a new client with an old engine during the upgrade is exactly the skew a daemon introduces; reads
-are harmless, writes against an unknown engine are not. The compat adapter is tested by running
-the unchanged v0.1 TUI (`native.py`) against the v0.2 daemon (P1-10).
+A new CLI that finds only an old, TUI-hosted `engine.sock` (no `api.sock`) still reads through it
+but refuses writes with "an older fbittorrent app is running; quit it with Q and retry". Why: mixing
+a new client with an old engine during the upgrade is exactly the skew a daemon introduces; reads
+are harmless, writes against an unknown engine are not. The compat adapter is tested by running
+the pinned v0.1.2 CLI and widget against the v0.2 daemon (P1-10); the v0.1 TUI owns its engine and
+never attaches (`src/main.rs:787`), so after installing v0.2 the only TUI is the new one (P1-15).
```

### A3. `events --since-seq N` is unimplementable: no event carries a sequence number and nothing retains events

- **Location:** P1-18 (line 3115), §6.3 row "Agent event stream" (line 2391), Appendix A.8
  (lines 4187-4190), Appendix F `events` row (line 4661).
- **What is wrong:** A.8 says unsolicited events "do not carry `rev`" and are sent only to
  connected clients. A `--since-seq` flag implies a daemon-global, monotonic event sequence and a
  replay buffer. Neither exists in the catalogue. An agent that reconnects after a network blip or a
  daemon restart therefore cannot ask "what did I miss?", which is the one thing `events` is for.
- **Fix:** give every unsolicited event a daemon-global `seq` (per instance), keep a ring of the
  last 1,000 events for 10 minutes, and make `events` a topic so the normal subscribe machinery
  (snapshot = replay, then live) carries it.

```diff
@@ PLAN.md:4161 @@
 | `search:<id>` | keyed | `key` | `SearchResult` | 500 ms | search views |
+| `events` | keyed, append-only | `seq` | the unsolicited events of A.8 (`torrent.*`, `engine.changed`, `mutation.result`, `search.done`, `shows.checked`, `clipboard.magnet`) wrapped as `{seq, ts, ev, …}` | on change | `events --follow`, agents |
+
+Subscribing to `events` with `since_seq: u64?` replays the retained ring from that sequence (the
+snapshot), then streams live. The ring keeps 1,000 entries or 10 minutes, whichever is smaller;
+a `since_seq` older than the ring answers `snapshot.begin {reason: "resync"}` with
+`details.oldest_seq` in the result so the client knows it missed events. `seq` restarts at 1 with
+every `instance`, so a client stores `(instance, seq)`.
```

```diff
@@ PLAN.md:4187 @@
-Unsolicited events (`torrent.*`, `engine.changed`, `mutation.result`, `search.done`,
-`shows.checked`, `clipboard.magnet`, `heartbeat`, `bye`) are sent to every connection that has
-completed `hello`, whether or not it subscribed to a topic; they are not coalesced and do not carry
-`rev`.
+Unsolicited events (`torrent.*`, `engine.changed`, `mutation.result`, `search.done`,
+`shows.checked`, `clipboard.magnet`, `heartbeat`, `bye`) are sent to every connection that has
+completed `hello`, whether or not it subscribed to a topic; they are not coalesced and do not carry
+`rev`. Every unsolicited event except `heartbeat` and `bye` carries `seq: u64` (daemon-global,
+monotonic per instance) and `ts`, and is also appended to the `events` topic (A.7) so a client can
+replay what it missed with `since_seq`.
```

```diff
@@ PLAN.md:3115 @@
-- *Do:* `fbittorrent events [--follow] [--since-seq N] [--kinds a,b] --json` connects, subscribes to
-  `summary` plus the event kinds asked for, and prints one JSON object per event on stdout (exit 0
-  at EOF or on SIGINT, 4 when the daemon is down, 2 on a bad kind); `fbittorrent api <op>
+- *Do:* `fbittorrent events [--follow] [--since-seq N] [--kinds a,b] --json` connects, subscribes to
+  the `events` topic with `since_seq` (A.7), filters by kind client-side, and prints one JSON object
+  per event on stdout; without `--follow` it prints the replay and exits 0; with `--follow` it
+  continues until EOF or SIGINT (exit 0). Exit 4 when the daemon is down, 2 on a bad kind. Each
+  printed event carries `seq` and `instance` so an agent can resume with
+  `--since-seq`; `fbittorrent api <op>
```

```diff
@@ PLAN.md:4661 @@
-| `events [--follow] [--since-seq] [--kinds] --json` (new) | `subscribe summary` + unsolicited events | no | new (NDJSON, one event per line) | 0, 2 (bad kind), 4 |
+| `events [--follow] [--since-seq] [--kinds] --json` (new) | `subscribe events {since_seq}` (replay, then live) | no | new (NDJSON, one event per line, each with `seq` and `instance`) | 0, 2 (bad kind), 4 |
```

### A4. A `Pieces` state frame can exceed the 1 MiB server-to-client line cap

- **Location:** Appendix A.9 `Pieces` (line 4202), A.7 topic `torrent:<h>.pieces` (state kind,
  line 4152), §6.3 P6-02 row (line 2398).
- **What is wrong:** `runs: [[start, length, have]]` is a run-length encoding; its worst case
  (alternating have/missing pieces, which a swarm-random download produces before sequential
  catch-up) is one run per piece. A 100,000-piece torrent (a 25 GiB season pack at 256 KiB pieces)
  serialises to about 1.4 MB of runs, over the cap of §3.4.1, and `state` frames cannot be chunked
  (§3.4.7 chunks only keyed snapshots). The daemon would have to drop the frame or close the client.
- **Evidence:** `PLAN.md:926-927` (1 MiB cap), `PLAN.md:1059-1061` (`state` sends the whole object).
- **Fix:** send the have-bitfield itself, base64-encoded: 100,000 pieces are 12,500 bytes, 16.7 KB
  in base64, bounded by construction, and the client needs no RLE decoder. Keep `priority`
  (first/last piece indices per file) and `playable_end`.

```diff
@@ PLAN.md:4202 @@
-| `Pieces` | `count: u64`, `piece_size: bytes`, `runs: [[start: u64, length: u64, have: bool]]`, `priority: [u64]` (first and last pieces per file), `playable_end: u64?` |
+| `Pieces` | `count: u64`, `piece_size: bytes`, `have: str` (the have-bitfield, MSB-first as in BEP 3, base64; `ceil(count/8)` bytes before encoding, so 100,000 pieces are 16.7 KB), `priority: [u64]` (first and last pieces per file), `playable_end: u64?` |
```

```diff
@@ PLAN.md:2398 @@
-| Pieces bar (P6-02) | Run-length encode the have bitfield (P5-06's accessor); mark first and last pieces per file; mark the playable prefix | `torrents.pieces {hash}` → `{count, piece_size, runs: [[start, len, have]], priority: [...], playable_end?}`; topic `torrent:<h>.pieces` | one-line bar in details | `PiecesBar` component | via `api` | Unit: RLE round-trip; fixture with 1,000 pieces |
+| Pieces bar (P6-02) | Base64 the have bitfield (P5-06's accessor); mark first and last pieces per file; mark the playable prefix | `torrents.pieces {hash}` → `{count, piece_size, have: base64, priority: [...], playable_end?}`; topic `torrent:<h>.pieces` | one-line bar in details | `PiecesBar` component | via `api` | Unit: bitfield round-trip and a 200,000-piece frame under 64 KiB; fixture with 1,000 pieces |
```

### A5. Wrong or imprecise citations (6 of 44 spot-checked), plus two factual slips

| PLAN line | Cited | Actual | Fix |
|---|---|---|---|
| 2333 | `vendor/rqbit/librqbit/src/session.rs:565` for "`Session.ratelimits` already has live setters" | line 565 is `let parent_cx = cx.clone();`. The field is `pub ratelimits: Limits` at `session.rs:140`; the setters are `limits.rs:85-91` (correct in the same cell) | `session.rs:140` |
| 586, 667 | `src/config.rs:139-146` and `:143-146` for the XDG paths / runtime dir | `data_dir` is 140-142, `runtime_dir` is 144-147; 143 is blank | `140-147` and `144-147` |
| 284, 1270, 3055 | `src/setup.rs:112-114` for "the desktop entry runs `add %u`" | `add %u` is on line 115 (`fbittorrent-magnet.desktop`); line 112 is `fbittorrent.desktop` with `Exec={command} up` | `111-115` |
| 676 | "the existing reasoning at `src/ipc.rs:181-185`" | 180-185 is bare `remove_file` code with no comment; the reasoning ("The stable lock file must never be unlinked while another app can open it") is the comment at `src/ipc.rs:147` | `src/ipc.rs:147` |
| 265 | `capabilities_json` (`src/agent.rs:134-205`) | the function spans 134-210 | `134-210` |
| 1151 | "Concurrent search sessions 4 — Each runs up to 18 Python providers" | nova2 runs all engines inside one Python process with a `spawn` pool of `min(cpu_count, 4)` workers (`vendor/nova3/nova2.py:71-75, 262-264`); a session is at most five processes | see hunk |
| 3789-3790 | topological level table puts M6 at level 18 | M6's deepest dependency is P7-04 (level 16), so M6 is level 17 beside M3 and M5 (Section D) | see hunk |
| 1610 | "Lines longer than 1 MiB are dropped and the session reset" (QML can bound memory) | `SplitParser` has no length cap; `DataStream::onBytesAvailable` appends everything until the delimiter (`quickshell-src/src/io/datastream.cpp:37-41, 91-98`). The client can only check `message.length` after the split; the server cap is the bound | see B11 |

```diff
@@ PLAN.md:2333 @@
-| 5 | Global speed limits | none (`src/engine.rs:417-439`) | In | T1 | Streaming next to a video call; `Session.ratelimits` already has live setters (`vendor/rqbit/librqbit/src/session.rs:565`, `limits.rs:85-91`). | P5-04 |
+| 5 | Global speed limits | none (`src/engine.rs:417-439`) | In | T1 | Streaming next to a video call; `Session` exposes `pub ratelimits: Limits` (`vendor/rqbit/librqbit/src/session.rs:140`) with live setters `set_download_bps`/`set_upload_bps` (`limits.rs:85-91`), so no fork change is needed. | P5-04 |
```

```diff
@@ PLAN.md:1151 @@
-| Concurrent search sessions | 4 | Each runs up to 18 Python providers |
+| Concurrent search sessions | 4 | Each is one nova2 process plus a `spawn` pool of `min(cpu_count, 4)` workers that run the enabled providers (`vendor/nova3/nova2.py:71-75,262-264`): at most about 5 processes and 150 MB per session |
```

```diff
@@ PLAN.md:3789 @@
-| 17 | M3, M5 |
-| 18 | M6 |
+| 17 | M3, M5, M6 |
```

Other spot-checks were exact: `src/main.rs:288,314-318,407-432,452-455,518-523,525-532,550-556,583-605,627-641,775-778,786-827,1022,1296-1300`; `src/ipc.rs:1-8,26-27,29-56,62-85,87-122,136-164,174-263,278-286`; `src/engine.rs:32,38-53,63-65,90-92,213-219,280-343,301-310,395-499,403-409,417-439,437,455-471,484-487,517-546,549-571,577-583,586-651,648-649,657,753-818,756-760,766-776,830-848,1233-1355,1396-1440,1474-1486`; `src/qbt.rs:40-44,100-102,223-237,295,413-423,504-520,547-627,706`; `src/agent.rs:7,12-29,113-123,186,221,296-384`; `src/tui/mod.rs:141,309-350,353-375,779-794,796,1160,1178,1207-1212,1255-1262,1325-1336,1346-1348,1664-1674,1699-1702,2040,2148,2199,2223,2229,2642,2770-2785,2933-2949,2979-2997,3000,3023`; `src/search.rs:26-27,37-47,583-688,723-738`; `src/shows.rs:62-64,115,246-262`; `src/imdb.rs:1-4,13,101-115,117-183`; `src/browse.rs:31-44,60`; `src/tui/theme.rs:57-79,66-67,74-77`; `src/tui/poster.rs:12-14`; `Service.qml:62-65,78,80-118,121-126`; `Panel.qml:18,72-76,119-122,124-139,145,149-150,162-163,179,223-226,311-330,332-460,395-412,410,433,442,450-459`; `Model.js:32-57,79`; `manifest.json:18-19,23-28`; `vendor/rqbit/librqbit/src/{session.rs:567-570, file_info.rs:14-25, limits.rs:85-91, chunk_tracker.rs:205, torrent_state/mod.rs:186,266, initializing.rs:113-114, live/mod.rs:217,782, live/peer/stats/snapshot.rs:27-32}`; `librqbit-core/Cargo.toml:30`; `librqbit/Cargo.toml:33`; `Cargo.toml:34,49-52,54-57`; `all.sh:6,12-16`; `native.py:44-50`; `README.md:36-37,61-69,79-86,134-135,162,213-235,271`; ftui `terminal_session.rs:107,147-167,1270-1295`, `program.rs:111-117`; Omarchy `shell.qml:931,1161-1172,1333,1356-1372,1453`, `PluginRegistry.qml:118-133,148-163`, `PluginShellApi.qml:29-31`, `Util.qml:77-79`, `ConfirmDialog.qml:12`.

### A6. The existing `autostart` config key disappears without a rule

- **Location:** §3.2.3 (table, lines 634-644), Appendix A.9 `Settings` (line 4206), P1-11.
- **What is wrong:** today `autostart = false` means "commands require an already open app"
  (`src/config.rs:39`, `src/backend.rs:20-21`, README:164-169, and the agent contract's
  `native_connect` rule at `src/agent.rs:182`). The plan introduces `--no-start`,
  `FBITTORRENT_NO_START` and `stop_policy` but never mentions `autostart`. A fresh agent will
  either delete the key (breaking the owner's config and the 1.x contract text) or leave it dead.
- **Fix:** keep the key with its existing meaning as the persistent form of `--no-start`.

```diff
@@ PLAN.md:644 @@
 | Any command with `--no-start` or `FBITTORRENT_NO_START=1` | never | Exit code 4 with a hint. |
+| Any command when the config has `autostart = false` (the existing key, `src/config.rs:39`, README:164-169) | never, except `up`, `tui`, `dashboard` and `serve`, which are explicit | Exit code 4 with the hint `fbittorrent up`; this is the persistent form of `--no-start` and keeps today's documented meaning (`src/agent.rs:182`). |
```

```diff
@@ PLAN.md:4206 @@
-| `Settings` | `max_active_downloads: u64`, `peer_encryption: enum(off\|prefer\|require)`, `network_interface: str?`, `save_path: str`, `seed_by_default: bool`, `stop_policy: enum(idle\|last-ui\|explicit)`, `idle_exit_secs: u64`, `notify: bool`, `handler: enum(add\|confirm)`, `clipboard_watch: bool`, `peer_port: u64`, `show_quality: str`, `download_limit_bps?`, `upload_limit_bps?`, `alt_download_limit_bps?`, `alt_upload_limit_bps?`, `restart_class: [str]` (the keys whose change restarts the engine) |
+| `Settings` | `max_active_downloads: u64`, `peer_encryption: enum(off\|prefer\|require)`, `network_interface: str?`, `save_path: str`, `seed_by_default: bool`, `autostart: bool`, `stop_policy: enum(idle\|last-ui\|explicit)`, `idle_exit_secs: u64`, `notify: bool`, `timeout_secs: u64`, `handler: enum(add\|confirm)`, `clipboard_watch: bool`, `peer_port: u64`, `show_quality: str`, `download_limit_bps?`, `upload_limit_bps?`, `alt_download_limit_bps?`, `alt_upload_limit_bps?`, `restart_class: [str]` (the keys whose change restarts the engine) |
```

### A7. `register-magnet` is a CLI command missing from Appendix F (and from today's `capabilities`)

- **Location:** Appendix F (lines 4624-4666); P1-11.
- **What is wrong:** `Cmd::RegisterMagnet` exists (`src/main.rs:196`, README:131) and Appendix F
  claims a 1:1 mapping of every command. The installed `capabilities --json` already omits it
  (`evidence/installed-capabilities.json` lists 27 commands without `register-magnet` or
  `add-clipboard`); P1-11 should close both gaps since it rewrites `capabilities` for 1.2.0.

```diff
@@ PLAN.md:4665 @@
 | `setup [--magnet] [--save] [--plugin=link\|copy] [--dry-run] [--uninstall] [--purge]` | none (files, systemd, shell IPC) | no | extended (`actions[]`) | 0, 1, 3 (`--purge` without `--yes`) |
+| `register-magnet` | none (`xdg-mime default`, desktop entries; the same step as `setup --magnet`) | no | unchanged | 0, 1 |
```

```diff
@@ PLAN.md:3027 @@
   `dashboard`, `log`; a deprecation warning when `engine = "qbittorrent"`; `robot-docs` and
-  `capabilities` 1.2.0; `agent_contract.py` (the successor of `run.sh`: every R-case rewritten
+  `capabilities` 1.2.0, whose `commands[]` now lists every clap subcommand including the two the
+  1.1.0 list omits (`add-clipboard`, `register-magnet`; a unit test derives the list from the
+  `Cmd` enum so it cannot drift again); `agent_contract.py` (the successor of `run.sh`: every R-case rewritten
```

### A8. Two exit-code mappings contradict the dictionary the plan promises to keep

- **Location:** A.3 `busy` row (line 4055); P1-11 `status --quiet` (lines 3025-3026); Appendix F
  `status` row (line 4628); reachability paragraph (lines 4668-4671).
- **What is wrong:** exit 1 is defined as "Something unexpected (bug, I/O). Retrying rarely helps"
  (`src/agent.rs:14-15`, `capabilities.exit_codes[1]`). `busy` is `retryable: true` by the plan's
  own table, and `status --quiet` exiting 1 for "the daemon is stopped" is the "exit 1 means no
  result" anti-pattern the agent-ergonomics kernel forbids (Axiom 5). Code 4 ("torrent app
  unavailable; run `fbittorrent up`") is the right class for both.

```diff
@@ PLAN.md:4055 @@
-| `busy` | a limit was reached (connections, sessions, dispatch queue) | yes | `{limit}` | 1 |
+| `busy` | a limit was reached (connections, sessions, dispatch queue) | yes | `{limit}` | 4 (the hint says "retry in a moment"; 1 would contradict `retryable`) |
```

```diff
@@ PLAN.md:3025 @@
-  version-skew warnings; the new `status` shape and `status --quiet` (no output; exit 0 when
-  running, 1 when stopped, for the menu gate of §8.6); `--no-start`; `serve`, `restart`,
+  version-skew warnings; the new `status` shape and `status --quiet` (no output; exit 0 when
+  running, 4 when stopped, for the menu gate of §8.6, which only tests non-zero); `--no-start`; `serve`, `restart`,
```

```diff
@@ PLAN.md:4628 @@
-| `status [--quiet]` | `hello`, `transfer.info`, `torrents.list` | no | **changed** (DL-22): `ok: true, running: false` when stopped; `daemon`, `history`, `all_time` added | 0; `--quiet`: 0 running, 1 stopped |
+| `status [--quiet]` | `hello`, `transfer.info`, `torrents.list` | no | **changed** (DL-22): `ok: true, running: false` when stopped; `daemon`, `history`, `all_time` added | 0; `--quiet`: 0 running, 4 stopped |
```

```diff
@@ PLAN.md:4668 @@
-Reachability of every CLI exit code: 1 (`start` on `done`; `internal`), 2 (`seq`; `api nope`;
-bad flag), 3 (`delete --files` without `--yes`), 4 (`--no-start` with a stopped daemon; `events`
-when down), 5 (`search` provider failure), 6 (`show zzzz`), 7 (an ambiguous selector), 8 (`wait
---timeout`). `agent_contract.py::exit_code_dictionary_total` exercises each one.
+Reachability of every CLI exit code: 1 (`start` on `done`; `internal`), 2 (`seq`; `api nope`;
+bad flag), 3 (`delete --files` without `--yes`), 4 (`--no-start` with a stopped daemon; `events`
+when down; `status --quiet` when stopped; `busy`), 5 (`search` provider failure), 6 (`show zzzz`),
+7 (an ambiguous selector), 8 (`wait --timeout`). `agent_contract.py::exit_code_dictionary_total`
+exercises each one, and `agent_contract.py::retryable_never_maps_to_1` asserts that no API error
+with `retryable: true` maps to exit 1.
```

### A9. What a client does when the daemon drains under its request is unspecified (idle exit race)

- **Location:** §3.2.4 `ensure_daemon()` (lines 650-657); §3.2.7 transitions to/from Draining
  (lines 732-733); §3.2.8 shutdown (lines 754-758).
- **What is wrong:** The idle countdown fires every 5 s independent of connections. A CLI that
  connects at T, sends `torrents.add` at T+10 ms and sees EOF (the daemon decided to exit at T-1 ms
  and sent `bye{idle}`) has no rule to follow: Draining does not say whether requests already read
  are answered, and `ensure_daemon()` retries `hello` exactly once after `systemctl start`, which
  returns immediately ("already active") when the unit is still deactivating, so the retry hits
  `ECONNREFUSED` and the CLI exits 4 for a daemon that was healthy a second ago. The plugin has
  rule 6 (resend non-destructive refs after reconnect) but the CLI has nothing, and the ref ring is
  per instance (DL-28), so a resend to the new instance applies fresh, which is correct for `add`
  (duplicate detection) and `start/stop/set_*` (idempotent) and must be refused for `remove`.
- **Fix:** a quiesce rule in the daemon and a one-retry rule in the client.

```diff
@@ PLAN.md:732 @@
-| any → Draining | idle countdown reaches 0 (§3.2.10); `app.shutdown`; SIGTERM/SIGINT/SIGHUP; panic (exit 70); binary replaced while idle (exit 75) | `bye {reason}` then socket close; `STOPPING=1` |
-| Draining → Absent | drain done or `TimeoutStopSec` | sockets unlinked, lock released, unit inactive (or restarting) |
+| any → Draining | idle countdown reaches 0 (§3.2.10); `app.shutdown`; SIGTERM/SIGINT/SIGHUP; panic (exit 70); binary replaced while idle (exit 75) | **Quiesce first:** the listener stops accepting; every request already read is answered (or answered `engine_unavailable {state: "stopping"}`, retryable, if it needs the engine); then `bye {reason}` is the last frame on every connection and the socket is closed; `STOPPING=1`. A connection that completes `hello` during Draining gets `state: "stopping"` and `engine_unavailable` on every op. |
+| Draining → Absent | drain done or `TimeoutStopSec` | sockets unlinked, lock released, unit inactive (or restarting) |
```

```diff
@@ PLAN.md:650 @@
 1. Connect to `api.sock` and send `hello` (200 ms connect timeout). Success ends here.
 2. If `--no-start` or `FBITTORRENT_NO_START=1`: exit 4, "fbittorrent is not running".
 3. If the unit file exists and `systemctl --user show-environment` succeeds: run
    `systemctl --user --no-ask-password start fbittorrent.service`, bounded to 30 s by the caller.
-   It returns when the daemon has sent `READY=1`. Retry `hello`.
+   It returns when the daemon has sent `READY=1`, or at once when the unit is already active.
+   Retry `hello`; on `ECONNREFUSED` or `state: "stopping"` the unit was exiting when `start`
+   looked at it, so wait for `systemctl --user is-active` to leave `deactivating` (poll 100 ms,
+   at most 10 s) and run `start` once more. Two failed rounds are a failure.
 4. Otherwise spawn `fbittorrent serve --detach` and poll `hello` for up to 15 s.
 5. On failure: exit 4 with the hint `journalctl --user -u fbittorrent -n 50` (or the fallback log
    path).
+6. **Mid-request loss.** If the connection ends (EOF, `bye {reason: idle|restart|upgrade}`) after a
+   request was sent and before its response: for a read, or for a mutation whose op is marked
+   idempotent in Appendix A (`start`, `stop`, `set_seed`, `set_files`, `set_upload_limit`, `add`,
+   `commit`, `discard`, settings), re-run steps 1-4 once and resend with the **same `ref`**; for
+   `remove`, `recheck`, `move` exit 8 (`timeout`) with the hint `fbittorrent list --json` and the
+   message "the daemon stopped while this request was in flight; check the list". Clients never
+   resend after `bye {reason: slow_client|hello_required|invalid_message}`.
```

### A10. The unit lacks `KillMode` and a start-rate-limit setting; the defaults fight the design

- **Location:** §3.2.2 unit text (lines 599-617).
- **What is wrong:** (1) The default `KillMode=control-group` makes systemd send SIGTERM to every
  process in the cgroup at once: the daemon **and** its nova2 pools, `systemd-inhibit`,
  `notify-send --wait` and `wl-paste --watch`. The daemon's own ordered teardown (process-group
  SIGKILL of providers, `src/search.rs:945-961`; inhibitor release) then races systemd, and
  `TimeoutStopSec=20` ends in SIGKILL. `KillMode=mixed` sends SIGTERM to the main process only and
  SIGKILL to the remainder at the timeout, which is what the drain path assumes.
  (2) The user manager on this machine has `DefaultStartLimitBurst=5` per `10s`
  (`systemctl --user show`). An on-demand unit that exits idle and is restarted by the next action
  is fine in daily use, but `daemon.py --systemd` with `idle_exit_secs = 2`, or a developer
  iterating with `restart`, will hit "start request repeated too quickly" and the unit refuses
  to start until `reset-failed`. The `RestartForceExitStatus=75` loop counts too.

```diff
@@ PLAN.md:599 @@
 [Unit]
 Description=fbittorrent torrent engine (started on demand)
 Documentation=https://github.com/odcpw/fbittorrent
+# On-demand units start often by design (idle exit, exit 75 upgrades, tests); the default
+# 5 starts per 10 s limit would otherwise wedge the unit until `systemctl --user reset-failed`.
+StartLimitIntervalSec=60
+StartLimitBurst=20
 
 [Service]
 Type=notify
 NotifyAccess=main
 ExecStart=/home/<user>/.local/bin/fbittorrent serve
 KillSignal=SIGTERM
+# SIGTERM to the daemon only; it drains providers, the inhibitor and notifier children itself.
+# Anything still alive at TimeoutStopSec is SIGKILLed by systemd.
+KillMode=mixed
 TimeoutStopSec=20
```

Also add to P1-04's *Accept*: "`systemctl --user show fbittorrent -p KillMode` prints `mixed`; ten
`start`/`down` cycles in 30 s leave the unit startable" and a `daemon.py::start_limit_not_hit` case.

### A11. The contract-stability claim and the `status` shape change contradict each other

- **Location:** §0.4 (line 178), §2.5 (line 494), P1-11 (line 3031 "additive"), Appendix F intro
  (lines 4620-4621) versus Appendix F `status` row (line 4628, "**changed**") and DL-22.
- **What is wrong:** flipping `ok` from `false` to `true` for the stopped case changes the meaning
  of an existing field within contract 1.x. The plan says both "changes are additive" and
  "changed". Either is defensible (the old shape violated the plan's own envelope rule: `ok:false`
  without `code`, `src/main.rs:410`), but the document must say which, and `capabilities` must
  declare it so agents can detect it.

```diff
@@ PLAN.md:4620 @@
-The CLI's JSON shapes and exit codes do not change for existing commands; the column "JSON shape"
-says whether the output is unchanged, extended (new optional fields only) or new. The column
-"Starts" is §3.2.3's rule.
+The CLI's JSON shapes and exit codes do not change for existing commands, with one declared
+exception: `status` while stopped answers `ok: true, running: false` instead of `ok: false`
+(DL-22), because the old shape was an error envelope without `code`/`code_name`
+(`src/main.rs:410`) that no agent following `robot-docs` could parse as success. `capabilities
+--json` 1.2.0 lists it under `changes: [{"since": "1.2.0", "breaking": true, "field":
+"status.ok", …}]`, keeps `backend: "stopped"` and `app_open: false` in the stopped shape so
+consumers of those fields keep working, and `robot-docs` says so. The column "JSON shape" says
+whether the output is unchanged, extended (new optional fields only), changed or new. The column
+"Starts" is §3.2.3's rule.
```

```diff
@@ PLAN.md:178 @@
-| `triage`, `capabilities`, `robot-docs`, exit codes 0..8 and `--json` stay; changes are additive (contract 1.2.0). | P1-11, Appendix F |
+| `triage`, `capabilities`, `robot-docs`, exit codes 0..8 and `--json` stay; changes are additive (contract 1.2.0) except the declared `status` stopped-shape fix (DL-22, Appendix F). | P1-11, Appendix F |
```

(Owner question C1 asks whether to accept this break or keep `ok: false` for one more minor.)

### A12. §3.2.3 offline ledger writes have no owning item, no test and no locking order

- **Location:** §3.2.3 row `stop`, `delete` (keep files) (line 637); P1-11.
- **What is wrong:** the behaviour is real work (the CLI must take `engine.lock`, re-read the
  ledger, write it with the parked path's fsync-and-rename, release) and racy against a daemon that
  is starting or draining, yet no item names it and no `daemon.py` case covers it. The daemon's 5 s
  lock retry (§3.2.5) only works if the CLI holds the lock for the whole read-modify-write.

```diff
@@ PLAN.md:637 @@
-| `stop`, `delete` (keep files) | no | Applied directly to the durable ledger while holding the instance lock, reusing the parked code path (`src/engine.rs:1233-1355`). Starting an engine only to pause something is pointless. |
+| `stop`, `delete` (keep files) | no | Applied directly to the durable ledger, reusing the parked code path (`src/engine.rs:1233-1355`). Order: `hello` fails with `ECONNREFUSED`/`ENOENT` → acquire `engine.lock` (`LOCK_EX\|LOCK_NB`, retried for 2 s; if it stays held the daemon is starting or draining, so retry `hello` instead) → read the ledger → write it (temp, fsync, rename, dir fsync) → release. The daemon's own 5 s lock retry at start covers the overlap. Starting an engine only to pause something is pointless. |
```

```diff
@@ PLAN.md:3036 @@
-- *Tests:* `agent_contract.py::{R-001…R-0xx (ported), exit_code_dictionary_total, reads_never_start,
-  json_determinism, stdout_stderr_split, status_stopped_shape, version_skew_warning}`.
+- *Tests:* `agent_contract.py::{R-001…R-0xx (ported), exit_code_dictionary_total, reads_never_start,
+  json_determinism, stdout_stderr_split, status_stopped_shape, version_skew_warning,
+  old_engine_sock_reads_only}` (the last runs the new CLI against a v0.1 TUI-hosted `engine.sock`
+  and asserts reads succeed and writes are refused, §3.6); `daemon.py::{offline_stop_ledger,
+  offline_delete_keep_files_ledger, offline_write_waits_for_draining_daemon}` (the last starts
+  `down` and an offline `stop` together and asserts the ledger holds both intents afterwards).
```

### A13. Commit C3 is not a coherent state, and the git-fork dependency needs an offline story

- **Location:** §8.1 commit table (lines 2495-2503); §8.2; P0-02.
- **What is wrong:** C3 commits a `[patch.crates-io] asupersync = { git = … }` entry and a
  `Cargo.lock` on top of the legacy `main` manifest, which has no `asupersync` dependency (it is
  pulled in by `librqbit`/`rqbit-runtime`, which arrive in C4). Cargo warns "patch … was not
  used" and `--locked` fails because the lock names crates the manifest does not reference. The
  table marks C3 "n/a (not yet referenced)", but a lockfile that disagrees with its manifest is a
  broken commit, not an unreferenced one. Fold C3 into C4. Separately, a `git` dependency means
  every `cargo build --locked` fetches GitHub once per fresh checkout and CI; `--offline` builds
  need `cargo vendor` or a `net.git-fetch-with-cli` note. The fallback (commit the 57 MB tree) is
  already option B; the plan should also record the third option it did not consider, which is
  the cheapest: `cargo package` the patched crate into `vendor/asupersync-0.5.0-fbt1.crate` (a
  few MB, since the 57 MB includes `examples/`, `assets/` and tests) and use a `[patch]` with a
  `path` to the unpacked crate without those directories.

```diff
@@ PLAN.md:2499 @@
-| C3 `build: Asupersync 0.5.0 timer-wheel patch from a pinned fork` | `Cargo.toml` `[patch.crates-io]` change and `Cargo.lock` | n/a |
-| C4 `app: native engine, IPC, VPN binding, queue, encryption, Shows, RSS, search, TUI` | all of `src/`, the remaining `Cargo.toml` and `Cargo.lock` changes, `README.md` | yes, first buildable commit |
+| C3 `app: native engine, IPC, VPN binding, queue, encryption, Shows, RSS, search, TUI` | all of `src/`, `Cargo.toml` (dependencies, workspace members, and the `[patch.crates-io] asupersync = { git = "…", rev = "<sha>" }` entry from P0-02), `Cargo.lock`, `README.md`, `CHANGELOG.md` starting at v0.1.1 (§8.5). One commit because the manifest, the lock and the sources that use them are one change; a `[patch]` for a crate the legacy manifest does not depend on is not a buildable intermediate state. | yes, first buildable commit |
```

(Renumber C5-C7 to C4-C6; the message of the former C3 moves into C3's body as a paragraph.)

```diff
@@ PLAN.md:2521 @@
-Fallback if the owner does not want a separate repository: commit `vendor/asupersync` as its own
-commit (option B). A git submodule is rejected: it complicates clones and builds and offers nothing
-over a git dependency.
+Fallback if the owner does not want a separate repository: option B, commit the patched crate
+**as `cargo package` would publish it** (`vendor/asupersync/` without `examples/`, `assets/` and
+the test fixtures that make up most of the 57 MB; measure with `cargo package --list`
+before deciding) as its own commit. A git submodule is rejected: it complicates clones and builds
+and offers nothing over a git dependency. With the fork, CI and `install.sh` need network for the
+first fetch; `cargo build --locked --offline` works after one online build, and the README says so.
```

---

## B. Improvements

### B1. Push state changes from the engine instead of discovering them on the next tick (P1)

- **Analysis:** §3.4.7 promises structural events "go out immediately", but §3.10 says the bus
  "takes one `torrents` snapshot per tick at the smallest active interval" (250 ms at best,
  1,000 ms by default), so "immediately" means "within the next tick". The fork already has a
  per-torrent `state_change_notify: Notify` (`vendor/rqbit/librqbit/src/torrent_state/mod.rs:212`,
  notified on every state transition at `:333, :412-460, :518-525`) that is `pub(crate)`. One
  small fork change, `pub fn state_changed(&self) -> impl Future` on `ManagedTorrent`, lets the bus
  wake on transitions and emit `delta`/`torrent.completed`/`torrent.error` within a few
  milliseconds while rates stay coalesced. It also removes the 1 s queue-loop latency on
  completion (`src/engine.rs:301-304`) for notifications. This belongs to P1-08 (the fork test
  harness exists via P0-05 → M0).

```diff
@@ PLAN.md:1062 @@
 - `interval_ms` is clamped to 250..10000 (default 1000). Structural changes (added, removed, state
   change, completion, playable, error, network change) go out immediately; rate and progress
-  changes are coalesced to the interval. Why: rates are noisy; state changes should feel instant
-  [FO §7.2].
+  changes are coalesced to the interval. "Immediately" is event-driven, not tick-driven: the bus
+  waits on each live torrent's `ManagedTorrent::state_changed()` (a fork accessor over the
+  existing `pub(crate) state_change_notify`, `vendor/rqbit/librqbit/src/torrent_state/mod.rs:212`)
+  and on the ledger's change counter, and emits the structural frame on wake; the per-tick
+  snapshot only carries rates and progress. Why: rates are noisy; state changes should feel
+  instant [FO §7.2], and a 1,000 ms default tick would make "pausing…" linger for a second.
```

```diff
@@ PLAN.md:2984 @@
 - *Do:* §3.4.7: `subscribe` and `unsubscribe`; topics `summary`, `torrents`,
   `torrent:<h>.files`, `network`, `settings`; chunked snapshots; deltas; immediate structural
-  events; coalescing; revisions; heartbeats; closing slow clients with `bye`; `engine.changed` with a
-  fresh snapshot; detail-topic release. Measure the budget (§3.10) and write `daemon-budget.md`
-  and the resource baseline.
+  events driven by a fork accessor `ManagedTorrent::state_changed()` over `state_change_notify`
+  (`torrent_state/mod.rs:212`; a fork unit test asserts it wakes on pause, unpause, completion
+  and error); coalescing; revisions; heartbeats; closing slow clients with `bye`; `engine.changed`
+  with a fresh snapshot; detail-topic release. Measure the budget (§3.10) and write
+  `daemon-budget.md` and the resource baseline.
```

### B2. Snapshot atomicity and ordering invariants (P1)

- **Analysis:** §3.4.7 does not say that the chunks of one snapshot come from one consistent
  state, or that no `delta` for the same subscription is sent between `snapshot.begin` and
  `snapshot.end`. Both are needed for `Store.js`'s reducer to be correct and testable; without
  them a client can apply a delta to a half-received snapshot. One sentence closes it.

```diff
@@ PLAN.md:1054 @@
   full objects never contain `null` (§3.4.9).
+- **Atomicity.** All chunks of one snapshot are taken from one consistent state under the ledger
+  lock; for a subscription no `delta` or `state` frame is sent between its `snapshot.begin` and
+  `snapshot.end`; changes that happen meanwhile arrive as the first `delta` after `snapshot.end`.
+  `Store.js` and `store.rs` may therefore apply chunks as they arrive and treat `snapshot.end`
+  as "consistent now".
```

### B3. Split P1-02 so the isomorphism test means something (P1)

- **Analysis:** P1-02 bundles (1) an isomorphic file split of `src/engine.rs`, (2) `EngineOp`
  typed dispatch replacing four route tables, and (3) `TorrentView` with the normalized state
  enum. The item's own test says "`torrents/info` JSON for a fixture ledger is byte-identical
  before and after", which is only true of (1); (2) and (3) change behaviour by design (the
  normalized model is new). P4-01 applies exactly this discipline to the TUI ("moves only").
  Splitting also lets P1-05 (contract types) start after (3) while (2) is still in progress.

```diff
@@ PLAN.md:2893 @@
-**P1-02 Engine split, typed operations, normalized model** (L)
-- *Do:* (1) Split `src/engine.rs` isomorphically into `src/engine/{mod,ledger,queue,ops,view,parked,legacy_import}.rs`.
-  (2) Add `enum EngineOp` covering every current route and `EngineDispatcher::execute(op)`; the
-  string routes become one parser used by the legacy socket. (3) Add `TorrentView` (§3.5) and the
-  single qBittorrent mapping for old clients. Remove the duplicated route lists
-  (`src/engine.rs:549-571`, `586-651`, `1233-1246`; `src/network_app.rs:78-90`).
+**P1-02 Engine split (isomorphic)** (M)
+- *Do:* Split `src/engine.rs` isomorphically into `src/engine/{mod,ledger,queue,ops,view,parked,legacy_import}.rs`.
+  Moves only; no behaviour change.
 - *Why:* API v1 needs typed operations; one table ends the four-place route sync [RC §10].
-- *Accept:* each route name appears once; old clients behave identically (`native.py`, all TUI
-  tests); every combination of rqbit state and record flags maps to exactly one `state`.
-- *Tests:* `view::tests::state_mapping_table` (table-driven over every `TorrentStatsState` ×
-  `paused` × `finished` × `stop_on_complete` × `record.paused` combination); an isomorphism test
-  that `torrents/info` JSON for a fixture ledger is byte-identical before and after; `native.py`.
+- *Accept:* `cargo test`, `native.py` and the `torrents/info` JSON of a fixture ledger are
+  byte-identical before and after; no file in `src/engine/` exceeds 900 lines.
+- *Tests:* the isomorphism test; `native.py`; `scripts/lint-sizes.sh` extended to `src/engine/`.
 - *Depends on:* P1-01.
+
+**P1-19 Typed operations and the normalized model** (M)
+- *Do:* Add `enum EngineOp` covering every current route and `EngineDispatcher::execute(op)`; the
+  string routes become one parser used by the legacy socket. Add `TorrentView` (§3.5) and the
+  single qBittorrent mapping for old clients. Remove the duplicated route lists
+  (`src/engine.rs:549-571`, `586-651`, `1233-1246`; `src/network_app.rs:78-90`).
+- *Why:* API v1 needs typed operations; one table ends the four-place route sync [RC §10].
+- *Accept:* each route name appears once; old clients behave identically (`native.py`, all TUI
+  tests); every combination of rqbit state and record flags maps to exactly one `state`.
+- *Tests:* `view::tests::state_mapping_table` (table-driven over every `TorrentStatsState` ×
+  `paused` × `finished` × `stop_on_complete` × `record.paused` combination); `native.py`.
+- *Depends on:* P1-02.
```

(Then P1-03, P1-05, P1-16, P5-06 and P6-07 depend on P1-19 instead of P1-02; §9.4 and the level
table shift by one level from level 7 on. Section D's script re-verifies in one run.)

### B4. Flip the slot to `Running` when the session exists; restore records in the background (P1)

- **Analysis:** §3.2.4 says `torrents.add` waits for `Running` up to 60 s and explains that a cold
  start "can spend minutes checking data (D3)". Today `engine::start` restores every record in
  series and awaits all of them before returning (`src/engine.rs:453-466`), with
  `concurrent_init_limit: Some(1)` (`:432`). P1-16 skips completed torrents, but every incomplete
  torrent (the ones the owner is actively downloading) is still re-checked before the slot turns
  `Running`, and under A1 that happens after every five idle minutes. librqbit admits new torrents
  while others initialise, so nothing requires the wait: make the slot `Running` as soon as
  `Session::new_with_opts` returns, spawn the restores as tasks that report per-record outcome
  (which P0-09 already needs for "one bad record does not block"), and let `torrents.add` proceed
  while others show `checking`. The magnet-click-to-"Added" latency after idle exit drops from
  "the sum of all checks" to under a second.

```diff
@@ PLAN.md:659 @@
-`READY=1` is sent once the API socket listens and the engine slot exists in `Starting`, not after
-hash checks finish. Operations that need a running engine wait for `Running` up to their own
-timeout (`torrents.add` waits up to 60 s), then fail with retryable `engine_unavailable`. Why: a
-cold start can spend minutes checking data (D3); the CLI and the bar must not block behind it.
+`READY=1` is sent once the API socket listens and the engine slot exists in `Starting`, not after
+hash checks finish. The slot becomes `Running` as soon as the librqbit session exists; saved
+records are restored by background tasks (P0-09 already makes each restore independent) and show
+as `checking` or `metadata` meanwhile, because librqbit admits new torrents while others
+initialise (`concurrent_init_limit` only serialises the checks, `src/engine.rs:432`). Operations
+that need the engine therefore wait only for the session (seconds, not minutes); `torrents.add`
+keeps a 60 s bound and fails with retryable `engine_unavailable` past it. Why: under A1 the
+daemon starts after every idle period, and today's `engine::start` awaits every restore in series
+(`src/engine.rs:453-466`), which would put the whole library's check time between a magnet click
+and "Added".
```

```diff
@@ PLAN.md:2778 @@
-- *Do:* In `engine::start` (`src/engine.rs:455-463`) restore records with cached metadata first;
-  a failure marks the record `error` (reason `restore_failed`, with the message), keeps it and
-  continues. Magnet-only records resolve in a background task with the 90 s timeout and show as
-  metadata meanwhile. `torrents/info` lists unrestored records from the ledger, as the parked path
-  already does (`src/engine.rs:1233-1355`). `start` on such a record retries.
+- *Do:* In `engine::start` (`src/engine.rs:453-466`) return as soon as the session exists and
+  restore every record in a spawned task (cached metadata first, then magnet-only records with the
+  90 s timeout); a failure marks the record `error` (reason `restore_failed`, with the message),
+  keeps it and continues. `torrents/info` lists unrestored records from the ledger as `checking`
+  or `metadata`, as the parked path already does (`src/engine.rs:1233-1355`). `start` on such a
+  record retries. (P1-03 reads the per-record outcome to drive the slot and `engine.changed`.)
```

### B5. Protocol robustness tests for the NDJSON reader (P1)

- **Analysis:** P1-06 tests one over-long line and one non-object line. A reader that will face
  agents' shell quoting and hot-reloading QML deserves a table: invalid UTF-8 mid-line, `\r\n`,
  an empty line, a 65,537-byte line that is otherwise valid, 10,000-deep nesting (serde_json's
  default recursion limit is 128; assert `invalid_message` not a stack overflow), duplicate keys,
  `id: 0`, `id` as a string, nine in-flight requests, a client that sends `hello` twice, a
  client that sends 1 byte per 100 ms to stall the hello deadline, and a client that connects
  and never reads while subscribed (the slow-client path, already in P1-08).

```diff
@@ PLAN.md:2961 @@
-- *Tests:* `daemon.py::contract_{hello,errors,torrents}` (each logs the JSON pointer of any
-  mismatch); `daemon.py::{connection_limit, line_cap, hello_deadline, non_object_line}`;
-  `api::server::tests::peercred_rejects_other_uid`.
+- *Tests:* `daemon.py::contract_{hello,errors,torrents}` (each logs the JSON pointer of any
+  mismatch); `daemon.py::{connection_limit, line_cap, hello_deadline, non_object_line,
+  hello_twice, ninth_inflight_busy, slow_hello_bytes}`; `api::server::tests::peercred_rejects_other_uid`;
+  `api::server::tests::garbage_table` (table-driven: invalid UTF-8, `\r\n`, empty line, 65,537
+  bytes, 10,000-deep nesting, duplicate keys, `id: 0`, string `id`, unknown top-level key; each
+  must answer `invalid_message`/`invalid_args` and never panic).
```

### B6. Make `api` self-describing: `api --list`, `robot-docs api`, and a compiled-in mutating table (P1)

- **Analysis:** Appendix F says `api <op>` starts the daemon "per op (`mutating` column)". When
  the daemon is down the CLI cannot ask it which ops mutate, so the table must be compiled in; the
  plan should say so and test it against Appendix A. The same table makes `api --list --json`
  (every op with args, result, mutating, since) and `robot-docs api` possible, which is the
  agent-ergonomics rule that a tool's contract is readable from the tool (Axiom 9) applied to the
  daemon split: today an agent would have to read `docs/IPC.md` to learn op names.

```diff
@@ PLAN.md:3117 @@
-  at EOF or on SIGINT, 4 when the daemon is down, 2 on a bad kind); `fbittorrent api <op>
-  [--arg k=v]… [--json-args '{…}']` sends one operation and prints the result envelope unchanged
-  (errors map to exit codes through Appendix A.3); both are listed in `capabilities` as
-  `json: true, mutating: <per op>` and in `robot-docs`.
+  at EOF or on SIGINT, 4 when the daemon is down, 2 on a bad kind); `fbittorrent api <op>
+  [--arg k=v]… [--json-args '{…}'] [--ref r]` sends one operation and prints the result envelope
+  unchanged (errors map to exit codes through Appendix A.3). `api --list [--json]` prints every
+  operation with its args, result, `mutating`, `idempotent` and `since` from `api::ops::TABLE`, the
+  compiled-in table the CLI also uses to decide whether an op may start the daemon when it is
+  down (`hello` is not reachable then); `robot-docs api` prints Appendix A. A unit test asserts
+  the table matches `docs/IPC.md`'s op list, and `api nope` suggests the nearest names from it.
+  Both verbs are listed in `capabilities` as `json: true, mutating: <per op>` and in `robot-docs`.
```

### B7. Client identity for the ref ring: include the pid in CLI and TUI names (P2)

- **Analysis:** §3.4.8 keys the ring by `(client.name, ref)` and has clients generate
  `<name>-<start-millis>-<counter>`. Agents run several `fbittorrent` processes in parallel; two
  CLIs named `cli` started in the same millisecond produce the same first ref, and the second one's
  mutation is silently answered with the first one's stored response. The fix is one rule.

```diff
@@ PLAN.md:1098 @@
 - Every mutating operation accepts `ref`. The daemon keeps the last 256 refs per `(client.name,
   ref)` pair per instance for 10 minutes. Repeating a ref returns the stored response without
   applying the operation again. Why keyed by client name: two clients that happen to pick the
   same ref must not collide; why per instance: §3.2.11. Clients generate refs as
-  `<name>-<start-millis>-<counter>`.
+  `<name>-<start-millis>-<counter>`, and a `client.name` must be unique per process for clients
+  that run many at once: the CLI and TUI use `cli-<pid>` and `tui-<pid>`; the plugin keeps the
+  fixed `oliver.torrents` because one shell process reconnects under the same name and must be
+  able to resend a pending ref after a reconnect (§4.4 rule 6). A unit test runs two clients
+  with identical refs and distinct names and asserts both mutations apply.
```

### B8. `hello` and `app.info` carry `pid`, how the daemon was started and where its log is (P2)

- **Analysis:** `ensure_daemon()` step 5 says the hint is `journalctl …` "or the fallback log
  path", but the client cannot know which applies to the daemon it just talked to (it might have
  been started by hand with `serve`, or detached). `fbittorrent log` works only while the daemon
  runs; after a crash the agent needs the right command. Three fields fix it and cost nothing.

```diff
@@ PLAN.md:4032 @@
-| result | `proto: u64`, `proto_minor: u64`, `server: {version: str, build: str}`, `engine: str`, `instance: str` (uuid), `engine_epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `caps: [str]` (§3.4.3), `limits: {line_in: u64, line_out: u64, inflight: u64, subs: u64, hashes: u64}`, `policy: {stop: enum(idle\|last-ui\|explicit), idle_exit_secs: u64}` |
+| result | `proto: u64`, `proto_minor: u64`, `server: {version: str, build: str, pid: u64, launcher: enum(systemd\|detached\|foreground), log: str}` (`log` is `journalctl --user -u fbittorrent` or the file path, ready to print as a hint), `engine: str`, `instance: str` (uuid), `engine_epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `caps: [str]` (§3.4.3), `limits: {line_in: u64, line_out: u64, inflight: u64, subs: u64, hashes: u64}`, `policy: {stop: enum(idle\|last-ui\|explicit), idle_exit_secs: u64}` |
```

The CLI remembers the last `server.log` it saw in `~/.local/state/fbittorrent/last-daemon.json` so
the exit-4 hint is right even when nothing answers.

### B9. Atomic poster-cache writes (P2)

- **Analysis:** `imdb::fetch_poster` writes the final path directly (`std::fs::write`,
  `src/imdb.rs:111`). After P3 the daemon and an in-process CLI share `~/.cache/fbittorrent/posters`
  and QML loads the files; a half-written JPEG renders as a broken image or a Qt warning on the
  shell's UI thread. Temp-and-rename is the pattern every store already uses.

```diff
@@ PLAN.md:3272 @@
-- *Do:* `imdb.lookup`; the ratings index loaded lazily once; posters cached as files; read caps
-  (2 MiB JSON, 8 MiB poster).
+- *Do:* `imdb.lookup`; the ratings index loaded lazily once; posters cached as files written to a
+  temporary name and renamed (today `std::fs::write` to the final path, `src/imdb.rs:111`, which a
+  concurrent in-process CLI lookup can expose half-written to QML); read caps (2 MiB JSON, 8 MiB
+  poster).
```

### B10. Bound the notifier (P2)

- **Analysis:** §3.8 runs "one short-lived thread per notification" with `notify-send -A … --wait`,
  which blocks until the notification closes (mako/swaync persistence can be minutes for
  `critical`). A bulk add of 50 torrents or a flapping tunnel is 50 blocked threads. Cap it.

```diff
@@ PLAN.md:1249 @@
-- **Notifier.** One short-lived thread per notification runs
+- **Notifier.** A queue drained by at most 4 threads (further notifications wait; the same
+  `(kind, hash)` pending twice is collapsed) runs, per notification,
```

### B11. Quickshell client rules: `flush()` after every write; honest line-length rule (P2)

- **Analysis:** `Socket::write` buffers into `QLocalSocket` and the header says "Remember to call
  flush after your last write" (`quickshell-src/src/io/socket.hpp:37-43`); without it a request
  can sit until the next event-loop turn. And per A5, the client cannot bound a partial line.

```diff
@@ PLAN.md:1604 @@
-2. `write()` adds no newline: always send `JSON.stringify(frame) + "\n"` [OR §6 gotcha 4].
+2. `write()` adds no newline and buffers: always send `JSON.stringify(frame) + "\n"` and call
+   `flush()` after it (`quickshell-src/src/io/socket.hpp:37-43`) [OR §6 gotcha 4].
```

```diff
@@ PLAN.md:1610 @@
-5. A `SplitParser` splits lines. Lines longer than 1 MiB are dropped and the session reset.
-   `JSON.parse` is wrapped in `try`.
+5. A `SplitParser` splits lines. The server's 1 MiB cap (§3.4.1) is the memory bound, because
+   `SplitParser` buffers until the delimiter with no cap of its own
+   (`quickshell-src/src/io/datastream.cpp:37-41,91-98`); a received line longer than 1 MiB means a
+   protocol violation and resets the session. `JSON.parse` is wrapped in `try`.
```

### B12. `torrents.add` by `.torrent` path knows the hash at once (P2)

- **Analysis:** §3.4.8 says the hash is "`pending` until metadata is known for URLs and files". A
  `.torrent` file contains the info dictionary; the daemon can hash it before answering (the
  engine's restore already parses files, `src/engine.rs:206-207`). Only URLs are pending. This
  matters for the handler path (`add %u` with a `file://` URL from the browser): "Added" with Undo
  needs the hash immediately.

```diff
@@ PLAN.md:1118 @@
-- `torrents.add` returns the info hash (`pending` until metadata is known for URLs and files).
+- `torrents.add` returns the info hash at once for magnets (from the `xt`) and `.torrent` paths
+  (the daemon bencodes-decodes the info dictionary before answering); only `{url}` sources are
+  `pending` until the download completes.
```

### B13. The Shows store's mtime re-read belongs to P3-03 too (P2)

```diff
@@ PLAN.md:3261 @@
-- *Do:* The `shows.*` operations and topics; the check scheduler; undo; `grab`; an EZTV fixture
-  server (`daemon.py` serves canned EZTV JSON).
+- *Do:* The `shows.*` operations and topics; the check scheduler; undo; `grab`; reload on
+  `(mtime, size, inode)` change of `shows.json` so an in-process CLI edit while the daemon runs
+  with `--no-start` appears in the topic (§3.7); an EZTV fixture server (`daemon.py` serves
+  canned EZTV JSON).
```

### B14. Seed flag: name the existing per-peer bitfield so P5-01 is a small fork change (P2)

- **Analysis:** §6.3 says "Fork: add a seed flag (complete bitfield) to `PeerStats`". The data
  already exists: `PeerStateLive.bitfield: BF` (`vendor/rqbit/librqbit/src/torrent_state/live/peer/mod.rs:259`)
  and an all-pieces check at `:298`. Citing them turns a vague fork task into a ten-line one.

```diff
@@ PLAN.md:2392 @@
-| Seed count and peer list (P5-01, P5-02) | Fork: add a seed flag (complete bitfield) to `PeerStats` (`torrent_state/live/peer/stats/snapshot.rs:27-32`); the daemon derives per-peer rates from successive `counters` samples | `torrents.peers {hash}`, topic `torrent:<h>.peers`; `seeds` (connected seeds) in `Torrent` | Details tab Peers | Details tab Peers; list column "seeds/peers" | `show <sel> --json` gains `seeds`, `peers[]` | Fork unit test with a local seed and a local leecher; fixture `peers.ndjson`; UI golden renders |
+| Seed count and peer list (P5-01, P5-02) | Fork: add `seed: bool` to `PeerStats` (`torrent_state/live/peer/stats/snapshot.rs:27-32`) from the live peer's `bitfield: BF` (`live/peer/mod.rs:259`; the all-pieces test at `:298` already exists) in the `From<&Peer>` impl at `snapshot.rs:57`; the daemon derives per-peer rates from successive `counters` samples | `torrents.peers {hash}`, topic `torrent:<h>.peers`; `seeds` (connected seeds) in `Torrent` | Details tab Peers | Details tab Peers; list column "seeds/peers" | `show <sel> --json` gains `seeds`, `peers[]` | Fork unit test with a local seed and a local leecher; fixture `peers.ndjson`; UI golden renders |
```

### B15. Test the two things the daemon split makes new about `wait` (P2)

- **Analysis:** `wait` today polls `torrents` (`src/main.rs:732-773`) and returns 8 when every
  waited torrent is stopped. With a daemon, two new cases exist: the daemon exits idle while `wait`
  runs (A1 says a torrent with intent run keeps it alive, so this means the torrent was paused by
  someone else: `wait` should then report the `stopped` case, not reconnect forever), and the
  daemon restarts for an upgrade (exit 75) mid-wait: `wait` should reconnect once and continue.
  Add `agent_contract.py::{wait_survives_upgrade_restart, wait_exits_8_when_daemon_idles_out}`
  to P1-11's tests.

---

## C. Questions only the owner can answer

- **C1.** `status --json` while stopped: accept the declared 1.2.0 break (`ok: true,
  running: false`, A11) now, or keep `ok: false` with the added `running` field until a 2.0.0?
  The plan assumes the break.
- **C2.** `autostart = false` is in your config schema today. Keep it as the persistent form of
  `--no-start` (A6, the plan's default after this review), or retire it in favour of `--no-start`
  only?
- **C3.** Asupersync hosting (existing Q3) gains a third option: a `cargo package`-shaped copy of
  the patched crate in-tree (B in A13), which is reproducible offline and far smaller than 57 MB.
  Fork, packaged copy, or full tree?

---

## D. DAG check output

Script: `scratchpad/review/dagcheck.py` (parses every `**P..** … *Depends on:*` item, the P6/P7
tables, the milestone lines and the §9.4 adjacency table; checks undefined IDs, cycles, orphans,
§9.3-versus-§9.4 disagreement, consumed-by consistency, the plan's level table, and the longest
path by item count and by size weight S=1, M=2, L=4). Output, verbatim:

```
items in §9.3: 106 (active 105); rows in §9.4: 105
no cycle; 105 nodes, 18 levels
orphans (no consumer): ['P7-08']
LEVEL differs M6: plan=18 computed=17
critical path to M1: 15 items, weight 35 (S=1,M=2,L=4): M1 <- P2-05 <- P2-03 <- P2-08 <- P2-01 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-03
critical path to M2: 16 items, weight 37 (S=1,M=2,L=4): M2 <- P3-16 <- P3-11 <- P3-09 <- P3-08 <- P2-01 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-03
critical path to M3: 17 items, weight 44 (S=1,M=2,L=4): M3 <- P4-10 <- P4-04 <- P4-03 <- P4-01 <- P1-15 <- P1-12 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-03
critical path to M4: 16 items, weight 40 (S=1,M=2,L=4): M4 <- P5-05 <- P4-03 <- P4-01 <- P1-15 <- P1-12 <- P1-08 <- P1-07 <- P1-06 <- P1-03 <- P1-02 <- P1-01 <- M0 <- P0-06 <- P0-15 <- P0-04 <- P0-03
sizes: Counter({'S': 43, 'M': 43, 'L': 12})
```

Reading: 105 active nodes (98 items plus 7 milestones; P7-07 superseded) match the plan's count.
No cycle. Every `Depends on` line in §9.3 agrees with the §9.4 adjacency table and every
"Consumed by" cell is the exact reverse of the dependency edges (no `DISAGREE` lines). The only
orphan is P7-08, which the plan declares conditional. The plan's level table is right except M6
(A5). The weighted critical path to M1 runs through P1-01 → P1-02 → P1-03 → P1-06 → P1-07 → P1-08
→ P2-01 → P2-08 → P2-03 → P2-05 (35 S-units; four L items in series), which supports B3 and B4:
the earliest user-visible change is at the end of P2, and nothing shortens that except splitting
P1-02 and running the plugin lane against P1-06's server before P1-08's bus is complete (the
popup's first version can poll `torrents.list` over `api.sock` for a day and switch to
`subscribe` when P1-08 lands; the plan may want to say so in §9.2's parallelism note).
