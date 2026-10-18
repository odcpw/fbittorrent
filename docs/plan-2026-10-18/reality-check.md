# fbittorrent reality check and UX audit (2026-10-18)

Scope: branch `codex/shows-native-engine` at `564fa18` plus its uncommitted working tree, the installed binary
`~/.local/bin/fbittorrent`, the live user configuration, and the Omarchy bar widget. Methods: the
`reality-check-for-project` Vision Checklist (Variant A) and the `ux-audit` Nielsen heuristics.
Nothing under `src/`, `plugin/`, `vendor/` or `Cargo.*` was modified. Evidence lives next to this file in
`evidence/` and `screens/`.

## 1. Executive verdict

1. The working tree is a working single-user torrent app. The native engine starts, re-verifies data, searches 18 providers, follows shows, reads RSS, and quits cleanly with `q`. 90/90 app tests and the isolated `native.py` CLI/TUI integration pass today.
2. **None of it is shipped in git.** `main`/`origin` (Sep 3) is still the legacy qBittorrent client. The native engine, IPC, Shows, RSS, Search, 60 MB `vendor/` and 279 MB of `docs/validation/` exist only as uncommitted files on one disk.
3. As a qBittorrent replacement, the feature surface is narrow. Of the 36 audited features, 9 exist, 9 are partial and 18 are missing: no peers or trackers view, no speed limits, no recheck, reannounce, move or rename, no completion notification and no log. Download CPU is still about 3.2× qBittorrent's (per the project's own 2026-10-13 receipts).
4. The engine cannot run without the TUI. It lives inside `fbittorrent tui`, which requires a TTY. A second frontend can drive the engine today only while that TUI process is open. The IPC is already TUI-independent, so a headless mode is a small, well-bounded change.
5. The UX is careful in places (contextual footer, scoped read actions, mouse support) but under-delivers on visibility. Below 140 columns the torrent state is shown only by color. On the user's monochrome theme, errors and successes look the same. Destructive prompts default to "yes". A widget timer can silently reopen the app after you quit it.

## 2. Shipped reality vs working-tree reality

| Layer | What it is | Evidence |
|---|---|---|
| `git` `main` / `origin/main` / HEAD `564fa18` (2026-09-03) | Legacy app: qBittorrent-nox WebUI backend, 3-screen TUI (Torrents/Search/RSS), "17 providers via qBittorrent's search plugins". Tracked `src/` has no `engine.rs`, `ipc.rs`, `shows.rs`, `rss.rs`, `search.rs`, `native_runtime*`. | `git ls-files src`; `git show HEAD:README.md` |
| Branch `codex/shows-native-engine` | **Zero commits beyond `main`.** Working tree: 19 modified files, +6363/−426, plus 3575 untracked files (new Rust modules, `vendor/` 60 MB with a patched Asupersync 0.5.0 and the rqbit 9.0.1 fork, `docs/validation/` 279 MB / 2197 files). `origin/ccr-1e478e53-vmcfrq` holds one unrelated "shows" commit (`eeb2055`). | `git status --porcelain`, `git diff --shortstat`, `du -sh` |
| Installed `~/.local/bin/fbittorrent` | **Native-engine build.** SHA256 `b7c6ede0…76b19c`, 21,555,840 bytes, Oct 14 05:55. That hash equals the one in `docs/validation/2026-10-14/launcher-fix/README.md`. `capabilities` contract 1.1.0, `engines.default = "native"`. No working-tree source file is newer than the binary. | `evidence/installed-capabilities.json`, `evidence/installed-help.txt` |
| Fresh `cargo build --release` of the tree | Builds clean in 8m21s (5 vendor `librqbit` warnings). Its `--help` and `capabilities --json` are **byte-identical** to the installed binary. Its hash differs because the installed copy was built with the `release-perf` profile. | `evidence/cargo-build-release.txt`, `evidence/worktree-capabilities.json` |
| Live config | `engine="native"`, `max_active_downloads=3`, `peer_encryption="prefer"`, `network_interface` unset (VPN binding **off**, although the 10-10 receipts installed "required ProtonVPN"), `save_path=~/Downloads`, `autostart=true`, `seed_by_default=false`. | `evidence/installed-config.json` |
| Live data (`~/.local/share/fbittorrent`) | `engine/torrents.json` (1.8 KB, 1 record, `migrated_legacy=true`), one cached `.torrent` (91 KB), `rss.json` 53 KB, `shows.json` 322 KB (mode **0644**, while the other stores are 0600), `search.json`, `nova3/` (Python helpers), `legacy-migration.json` 42 KB, `backups/` 56 MB (4 dated backups). Legacy `qbittorrent-nox.service` is disabled and inactive. | `ls -la` (names and sizes only) |
| Runtime dir | A **stale `engine.sock`** from an earlier 02:35 session was present with no process (ECONNREFUSED). The same thing reproduced today when the window was closed (see §5). | `evidence/app-lifecycle-observations.txt` |

**Bottom line:** the user runs the native build daily, but it lives only in an uncommitted working tree. A `git checkout`, `git clean`, or disk loss would revert the product to the September legacy app.

## 3. Vision Checklist

Sources: README.md (RM), docs/engine-evaluation.md (EE), docs/queue-encryption-vpn-plan.md (QV), docs/tui-ux-review.md (UX), and the owner's product decisions of 2026-09-03 (PD). The project has no beads (`.beads` is absent), so the NO_BEAD column is not applicable.

| # | Goal | Source | Status | Evidence |
|---|---|---|---|---|
| 1 | Embedded Rust engine runs only inside the open app; closing stops transfers, discovery, searches, feed checks | RM L5-7, PD | WORKING (caveat) | `q` exits 0 and removes the socket (`evidence/app-lifecycle-observations.txt`). `native.py`: "quit stops engine; restart preserves state; down closes app". Caveat: closing the terminal window kills the process non-gracefully and leaves a stale socket. |
| 2 | Sequential + first/last piece always on, not toggleable | RM L8-10, PD | WORKING (unit-level) | `vendor/rqbit/librqbit/src/file_info.rs:14-25` (first, last, then rest), toggles refused at `src/engine.rs:648-649`, capabilities `"sequential":"intrinsic"`. No fbittorrent test checks piece order on a real swarm. |
| 3 | Seeding off by default: 32 KiB/s cap while downloading, stop when complete; per-torrent `seed` | RM L11-16, PD | WORKING; docs WRONG | `src/engine.rs:833-848` and `868-872`; queue stops finished non-seeders at `src/engine.rs:306-310`. Live record: `upload_limit 32768`, `ratio_limit 0`, state `stoppedUP` after completion. But `capabilities`/`robot-docs` claim **"native: 1 B/s"** (`src/agent.rs:186`, `src/agent.rs:221`). The engine rejects caps below 16 KiB/s (`src/engine.rs:63-65`). |
| 4 | App on demand: CLI and widget open it when needed, `status` never opens it, nothing at login | RM L17-19, PD | PARTIAL (bug) | `status` never launches (probe). No autostart entries; legacy unit disabled. **But** the widget runs `fbittorrent reconcile` every 60 s (`plugin/oliver.torrents/Service.qml:121-126`), which goes through `connect()` and `ensure_running()` and calls `open_app()` (`src/main.rs:627-628`, `src/backend.rs:143-149`). Proven in isolation: `evidence/reconcile-autolaunch-probe.txt`. The widget's `online` flag can be up to 10 s stale, so the app can reopen after the user quits. |
| 5 | Bar widget with Wi-Fi-panel chrome: rates, progress rows, pause/remove, add from clipboard, one click to the TUI; visible only while the app is open; hover shows full text | RM L20-22, PD | WORKING per earlier receipts; UNPROVEN visually today | Native chrome: `Panel {` / `KeyboardPanel` / `PanelHero` / `BarIconButton` (`plugin/oliver.torrents/Panel.qml:10,141,155,202`). IPC `open` gave `{"opened":true,"rows":1}`. The popup could not be screen-captured within the time box (§5). Prior screenshots: `docs/validation/2026-10-10/native-cua/`. |
| 6 | TUI with 4 screens and IMDb card (rating, blurb, poster); `i`/`w` | RM L23-32 | WORKING | `screens/01-torrents-160x45.txt`, `screens/08-search-results-selected.txt`, `screens/12-shows-inbox.txt` |
| 7 | Hover preview, click select, double-click activate, wheel, marquee | RM L27-29, UX | WORKING | `native.py` mouse checks pass. A real SGR click on tab 4 and then tab 1 switched screens. Marquee observed (and criticised, §9). |
| 8 | Contextual bottom commands + separate feedback row | RM L30-31, UX | PARTIAL | Two-row footer exists (`src/tui/mod.rs:2014-2142`). Actions are silently dropped at 80 columns (`screens/21-torrents-80x24.txt`). |
| 9 | `?` scrollable help; `1`–`4`/Tab | RM L32 | WORKING | `screens/03-help-page{1,2,3}.txt` |
| 10 | Local show watchlist: EZTV episodes and season packs, check on open when >30 min, `r`/`R`, Enter downloads, durable read state, All-unread inbox, `m`/`u`/Shift+M | RM L33-49 | WORKING | `screens/12-shows-inbox.txt`, `screens/13-shows-one-show.txt` ("77 releases", "checked 25m ago"). 11 `shows::tests` and 21 TUI interaction tests pass; `native.py` Shows step passes. |
| 11 | Local RSS: cache, one refresh on open, `r`, Enter downloads and marks read, `m`/Shift+M | RM L50-55 | WORKING | Footer showed "feeds refreshed" on launch (`screens/01`). `screens/14`, `screens/15`; `native.py` RSS steps. |
| 12 | Provider browser (1337x browser-only; YTS, EZTV, TPB lists in TUI) | RM L56-60, L219-224 | WORKING; README stale | YTS "Latest movies: 50 entries" loaded inside the TUI via yts.lt (`screens/10-browse-yts-latest.txt`), contradicting README L221-223. |
| 13 | Provider picker `s`, stored locally | RM L61-64 | WORKING | `screens/11-providers-modal.txt` (18 of 18 enabled) |
| 14 | UPLOADED and IMDB columns; duplicates collapsed by info hash | RM L63-66 | PARTIAL | IMDB column present. UPLOADED/SOURCE/PEERS are hidden whenever the details pane is shown, even at 160 columns (`src/tui/mod.rs:3000,3023`; `screens/07`). Duplicates remain ("Sintel 2010 4k FLAC mkv" ×2, `screens/07-search-sintel-16s.txt`) because dedupe falls back to the full URL (`src/tui/mod.rs:2979-2997`). |
| 15 | Add from browser handler, clipboard, TUI `a` (+F4), CLI | RM L67-69 | WORKING (receipts), not re-run | `xdg-mime` magnet and `application/x-bittorrent` both map to `fbittorrent-magnet.desktop`. Receipts in `docs/validation/2026-10-09/desktop-integration.json`. No torrent was added in this audit (per instructions). |
| 16 | File selection: Space/1/a/n, no high/max | RM L70-72 | WORKING | `screens/02-files-modal.txt`; `src/engine.rs:653-703` |
| 17 | Download queue: 3 by default, FIFO, manual pause durable, F6 | RM L73-75, QV | WORKING | `src/engine.rs:280-343`; `native.py` "durable live queue limit"; `screens/04-settings-f6.txt` |
| 18 | Peer encryption prefer/require/off | RM L76-78, QV | WORKING (receipts) | `src/engine.rs:418-422`; libtorrent oracle receipts in `docs/validation/2026-10-10/encryption-tests.txt`. Today's `librqbit` lib run passed 97 tests; the libtorrent oracle is `#[ignore]` and was not re-run. |
| 19 | ProtonVPN binding with waiting state | RM L79-85, QV | PARTIAL / UNPROVEN on real Proton | Namespace fixtures pass (10-10 receipts; `native.py` "VPN waiting" passes today). Receipts say the real `proton0` was never connected for tests. Binding is currently **off** in the user's config. Error text hard-codes "ProtonVPN" even for other interfaces (`src/engine.rs:831,1248`). |
| 20 | Live theming from Omarchy | RM L86 | WORKING, poorly applied | `src/tui/theme.rs:57-79`, reload `src/tui/mod.rs:779-794`. Color-only semantics collapse on the active monochrome theme (§9). |
| 21 | Every CLI command takes `--json`; errors use an `{ok:false,…}` envelope with matching exit code | RM L104-107 | WORKING (minor inconsistency) | `capabilities` is valid. `status --json` returns `"ok":false` with exit 0 when closed (`src/main.rs:408-414`). |
| 22 | Local socket restricted to the user; no WebUI | RM L226-228 | WORKING | Dir 0700 and socket 0600 (`src/ipc.rs:143,187`). No peer-credential check. |
| 23 | Tests as documented | RM L237-254 | WORKING (app and engine) / REGRESSED doc commands | `cargo test --release`: 90 passed. `native.py`: passed. Engine libraries: 182 passed, 4 ignored, when run per crate. Both documented multi-crate library commands (`docs/validation/2026-10-10/README.md` and EE's `-p librqbit-core -p librqbit`) **fail to build** (sha1 feature unification, `evidence/cargo-test-engine-libs-combined-FAILED-BUILD.txt`); see §11. Legacy `run.sh`/`tui_smoke.sh`/`widget_keys.sh` need qBittorrent or a live shell and were not run. |
| 24 | No Tokio/hyper/reqwest in the normal graph | EE | WORKING | `cargo tree -e normal -p fbittorrent` shows 0 tokio/hyper/reqwest lines |
| 25 | Engine performance comparable to qBittorrent | EE (limits), validation 10-11..10-13 | PARTIAL (known gap) | Own receipts: download CPU 3.21×/3.16× qBittorrent after optimizations; RAM about 53% lower. Large-library startup re-hash is unmeasured. |
| 26 | Default terminal Ghostty | PD | WORKING (system-level) | `~/.config/xdg-terminals.list` → Ghostty; the app goes through `omarchy-launch-or-focus-tui` (`src/backend.rs:193-194`). Real launch opened `/usr/bin/ghostty --class=org.omarchy.fbittorrent`. |
| 27 | Downloads go to `~/Downloads` | PD | WORKING | `save_path` set; fallback `dirs::download_dir()` (`src/engine.rs:403-409`) |
| 28 | Keys documented in README | PD | PARTIAL | README covers screen keys and read actions, but not `p`/Space pause, `d`/`Shift+D` delete, `o` open, `c` clipboard, `O`, `x`, `q`/`Q`, `/`, `g`/`G`. It defers to `?`. |
| 29 | Widget uses native Panel chrome, not a tray icon | PD | WORKING | `Panel.qml:10,155`; manifest `kinds: ["bar-widget"]` |
| 30 | Shared action definitions, responsive narrow layouts, PgUp/PgDn everywhere (own UX review "next steps") | UX L176-201, L261-263 | NOT_STARTED | Hints are still hand-written per screen (`src/tui/mod.rs:2037-2104`). Lists have no PgUp/PgDn (only Help does, `src/tui/mod.rs:1169-1170`). RSS has no Home/End (`src/tui/mod.rs:1469-1528`). |

## 4. What works (proven today)

- **Build and tests:** release build OK (`evidence/cargo-build-release.txt`). `cargo test --release`: **90 passed, 0 failed** in 6.8 s (`evidence/cargo-test.txt`): TUI interaction 21, shows 11, search 11, legacy client 11, engine 10, rss 8, imdb 4, misc 14. User config and data hashes are identical before and after the test run. `native.py` against the fresh build: **all 11 steps ok** (`evidence/native-py.txt`): mode-1003 mouse, RSS add-once, read state, mouse at scroll offset, CLI fields, file selection, cap, delete safety, Shows inbox, settings/queue/VPN-waiting restart, quit/restart/down.
- **Real TUI session** (installed binary, tmux 160×45, then 80×24 and 60×20; 26 text captures, 8 ANSI color captures and 2 PNGs in `screens/`):
  - The engine started.
  - The one saved torrent re-hashed 44.9%→100% in about 7 s and settled to "done" with its 32 KiB/s cap and stop-on-complete.
  - Searching "Sintel" streamed 87→100 results from 18 providers with IMDb 7.4 ratings, poster card and Wikipedia blurb.
  - The YTS browse list loaded 50 entries.
  - The providers modal listed 18.
  - Shows inbox showed 2 unread; one show's history showed 77 releases ("checked 25m ago").
  - RSS showed 1 feed with 47 items.
  - F6 settings and the add prompt with F4 toggle work, and Escape cancels both.
  - The `q` quit is clean: exit 0, empty stderr, socket removed.
- **Real Ghostty window** via the Omarchy launcher opened with class `org.omarchy.fbittorrent` (`screens/tui-real.png`).
- **CLI contract** (`evidence/installed-*.json/txt`): 27 commands; exit-code dictionary; JSON envelope; status never launches the app.
- **Widget IPC** answers (`cursor`, `open`, `close`). The QML uses the shell's native components.

## 5. What does not work, or is only claimed

**Functional defects found**
1. **The widget can resurrect a closed app.** `Service.qml:121-126` runs `fbittorrent reconcile` every 60 s while `online`. `reconcile` calls `connect()` (`src/main.rs:627-628`), which goes to `ensure_running()` and then `open_app()` (`src/backend.rs:143-149`). Proven with a fake launcher in isolated XDG dirs (`evidence/reconcile-autolaunch-probe.txt`). In native mode `reconcile` has no work to do (seq/first-last are intrinsic, and nothing can add torrents outside the app), so the timer is pure risk.
2. **Closing the terminal window is not a graceful shutdown.** The process dies about 2.2 s later, the `AppEngine`/`Server` `Drop` paths never run (`src/engine.rs:517-545`, `src/ipc.rs:278-286`), and `engine.sock` is left stale. Reproduced today, and also present from the user's own 02:35 session. The app installs no SIGHUP/SIGTERM handling (`src/main.rs:288` handles only SIGPIPE). Impact is bounded: the next start unlinks the socket (`src/ipc.rs:181-185`), `status` copes, and the ledger is fsynced per change.
3. **"Resume" on a completed stop-on-complete torrent is a no-op that reports success.** The footer offers `(P) Resume` (`src/tui/mod.rs:2040`), the action reports "started" (`src/tui/mod.rs:1328`), and `reconcile_queue` keeps it paused (`src/engine.rs:306-310`). The same applies to CLI `start`.
4. **Peer statistics are wrong by construction.** `num_seeds` is hard-coded 0 and every connected peer is reported as a leecher (`src/engine.rs:1479-1481`). The TUI details read "0 seeds, N leechers" (`src/tui/mod.rs:2199`) and the widget shows "0s" (`plugin/oliver.torrents/Model.js:79`).
5. **The agent contract documents the wrong upload cap** ("native: 1 B/s", `src/agent.rs:186,221`); the engine uses 32 KiB/s.
6. **One bad record blocks startup.** `engine::start` restores every record with `?` and a 90 s metadata timeout each (`src/engine.rs:458-463`, `213-219`). A record whose cached `.torrent` is missing and whose magnet has no peers fails the whole app open.
7. **Every open re-hashes all data** (`fastresume: false`, `src/engine.rs:437`). Observed: the 3.4 GiB library needed about 7 s of "checking". A multi-season library would make every app open slow, and this is unmeasured (EE admits it).
8. **The documented library-test reproduction commands no longer build** when crates are combined. `librqbit-core`'s default `sha1-crypto-hash` meets `librqbit`'s `sha1-ring` (`vendor/rqbit/librqbit-core/Cargo.toml:30`, `vendor/rqbit/librqbit/Cargo.toml:33`). Per-crate runs pass: 182 tests (`evidence/cargo-test-engine-libs*.txt`).
9. **Search hygiene:** visible duplicates; provider mojibake ("è¾›ç‰¹å°”") is shown unrepaired (`screens/07`). The likely origin is the provider plugin's decoding; the app does nothing about it.

**Mock/stub scan** (keyword scan for TODO/FIXME/XXX/HACK/unimplemented!/todo!/stub/placeholder/mock/fake/"not supported"/unsupported/unavailable):
- `src/` and `plugin/` contain **zero** TODO/FIXME/`todo!`/`unimplemented!`. Every "unsupported"/"unavailable" hit is a deliberate capability refusal with a user message:
  - `src/engine.rs:615` setPreferences
  - `src/engine.rs:648-649` sequential/first-last toggles
  - `src/engine.rs:657` high/max priority
  - `src/engine.rs:780-782` and `880-882` ratio limits
  - `src/engine.rs:1002,1046` legacy import layouts
  - `src/main.rs:814` unknown IPC route
- `vendor/rqbit`: 25 upstream TODO lines.
  - `todo!()` in `librqbit-dht/src/peer_store.rs:206` (`garbage_collect_peers`, `#[allow(dead_code)]`, unreachable; the DHT peer store is never garbage-collected).
  - `unimplemented!` in a Miri-only test (`librqbit/src/read_buf.rs:2103`).
  - `expect("TODO")` at `librqbit/src/session.rs:1310` and `torrent_state/mod.rs:449` (would panic if metadata is absent).
  - Torrent creation code exists (`librqbit/src/create_torrent_file.rs`) but is not exposed.

**Claimed in docs but not true or not proven**
- README L221-223: "YTS … browser-only". It is not; the YTS list loads in-app.
- README L63-64 promises an UPLOADED column. It is hidden in the default wide layout.
- README L66 says duplicates are collapsed by info hash. This is only partly true.
- 10-10 receipts say ProtonVPN is required. The live config has binding off, and real `proton0` behavior was never tested.
- Widget behavior today (popup content, tooltips) is UNPROVEN in this session. The popup did not appear in a region capture. That capture showed unrelated window content and was deleted rather than stored.

## 6. Architecture map and separability

```
                       ┌──────────────── fbittorrent tui  (ONE process; needs a TTY: src/main.rs:452-458) ───────────────┐
 user keys/mouse ─────►│ FrankenTUI loop: TuiApp (src/tui/mod.rs:141-214, ~70 fields, 4 screens + 3 modals)              │
                       │   │  every action/poll = Cmd::task → qbt::Client (src/tui/mod.rs:353-375)                       │
                       │   │  in-process libs: rss.rs, shows.rs, search.rs (Python child procs), imdb.rs                 │
                       │   ▼                                                                                              │
                       │ qbt::Client (struct, not a trait; `if cfg.native()` per method, src/qbt.rs:279-366, 547-629)    │
                       │   │ native → ipc::request() over the Unix socket ── even to its own process ──┐                 │
                       │   │ legacy → HTTP to qbittorrent-nox WebUI                                     │                 │
                       │   ▼                                                                            ▼                 │
                       │ ipc::Server  (listener thread + ≤16 request threads, src/ipc.rs:174-263) ◄── $XDG_RUNTIME_DIR/fbittorrent/engine.sock
                       │   │ handler = network_app::controls() → EngineDispatcher::dispatch() (src/main.rs:812-815)      │
                       │   ▼   each request: runtime.block_on(dispatch_async) (src/engine.rs:577-583)                     │
                       │ AppEngine: Asupersync runtime (1 worker, 1-2 blocking; src/native_runtime.rs:44-50)              │
                       │   └─ librqbit Session (DHT, trackers, LSD, PEX, TCP peers, MSE, bind-to-device) + 1 s queue loop │
                       │ Lifecycle flags: network_app RESTART/CLOSE (statics), engine QUIT_REQUESTED, polled by TUI tick  │
                       │   (src/tui/mod.rs:1699-1702); run_native_app restart loop (src/main.rs:786-827)                 │
                       └──────────────────────────────────────────────────────────────────────────────────────────────────┘
   other processes:  `fbittorrent list/add/start/...` (CLI) ──► same socket      Quickshell widget ──► spawns CLI `status --json` every 2/10 s
                     `fbittorrent search/rss/shows` (no engine needed, file stores with locks)
```

**How clean is the engine/TUI boundary?** Cleaner than the file sizes suggest:
- `engine.rs` imports nothing from `tui`. Its only couplings are `config`, `ipc` and the `qbt` data types (`src/engine.rs:28-30`).
- The TUI never calls the engine directly. Every TUI read and write goes over the socket through `qbt::Client`, which makes the TUI already a pure IPC client.

The coupling is in **lifetime and supervision**, not data:
- **Who creates the engine:** `run_native_app` is reached only from `Cmd::Tui` (`src/main.rs:452-458`). It starts the engine, serves IPC, then blocks in `tui::run` and drops the engine when the TUI returns (`src/main.rs:804-820`).
- **Who supervises it:** the TUI's 2 s `Msg::Poll` checks `engine::quit_requested()`, `network_app::close_requested()` and `network_app::check_transition()` (`src/tui/mod.rs:1699-1702`). This is how `fbittorrent down`, `app/restart` and VPN loss reach the process.
- **Who runs "on open" work:** the TUI's `init` triggers the RSS refresh and the show checks (`src/tui/mod.rs:1664-1666`).
- **Attach is impossible:** a second `fbittorrent tui` while the app is open just focuses the existing window (`src/main.rs:787`).
- **No daemon or headless command exists.**

**Can a second frontend (Quickshell dashboard, GUI) drive the engine today without the TUI open?** **No.** With the TUI open, **yes**:
- Any same-uid process can speak the socket protocol, or shell out to the CLI as the widget does.
- QML cannot easily speak the 4-byte length-prefixed framing; a JSON-lines variant would fix that.
- There is no push channel, so every client must poll.

**What would have to change for a headless engine or daemon mode** (estimated small, about 150-300 LOC excluding tests):
1. Add `fbittorrent serve` (or `daemon`) that runs `engine::start` and `app.serve`, plus a supervision loop that does what `Msg::Poll` does today: quit/close/transition checks and restart on settings change. Move that loop out of `src/tui/mod.rs:1699-1702` into `run_native_app` so the TUI and the daemon share it.
2. Make `tui` attach as a client when `ipc::is_running` is true, instead of `backend::open_app()` (`src/main.rs:787`). Give the TUI two quit verbs: "close window" (detach) and "stop engine" (today `q` always calls `network_app::request_close`, `src/tui/mod.rs:1207`).
3. Make `up`/`open_app` spawn the daemon detached (or use a systemd user socket unit started on demand) instead of a terminal. This keeps the owner's "on demand, nothing at login" rule. Add an idle-exit policy, for example exit when no client and no active transfer for N minutes.
4. Move "on open" RSS and show checks from the TUI's `init` into the daemon, or keep them client-triggered by policy.
5. Make the IPC a real contract:
   - a protocol version field;
   - typed request/response enums shared by client and server, instead of string routes plus `Vec<(String, String)>`;
   - a subscribe/event stream (torrent deltas, completion) so widgets stop spawning a process every 2 s;
   - optional JSON-lines framing for QML;
   - an `SO_PEERCRED` uid check.
6. Remove or native-gate `reconcile` in the widget (defect §5.1).

## 7. IPC contract (`src/ipc.rs`, routes in `src/network_app.rs` and `src/engine.rs`)

- **Transport:** Unix stream socket `$XDG_RUNTIME_DIR/fbittorrent/engine.sock`, falling back to `<data_dir>/runtime` (`src/config.rs:144-147`). Directory mode 0700, socket mode 0600 (`src/ipc.rs:143,187`). A single-instance `flock` on `engine.lock` is taken before any I/O (`src/ipc.rs:136-164`). The stale socket is unlinked on bind (`src/ipc.rs:181-185`).
- **Framing:** one request per connection. A 4-byte big-endian `u32` length is followed by a JSON body, max 32 MiB each way (`src/ipc.rs:26,62-85`).
- **Request:** `{method: String, path: String, args: [[k,v],...], file: {name, bytes:[u8...]}|null}` (`src/ipc.rs:29-41`). `bytes` is serialized as a JSON number array, about 3.5× inflation for `.torrent` uploads.
- **Response:** `{value: String|null, error: String|null}` (`src/ipc.rs:52-56`). `value` is a JSON document encoded inside a string (double encoding), or `"Ok."`.
- **Versioning:** none. `app/version` returns `"native librqbit 9.0.1"` (`src/engine.rs:588`), not a protocol version. Routes deliberately mirror qBittorrent WebUI v2 names so `qbt::Client` serves both backends.
- **Auth:** filesystem permissions only; no peer-credential check. The method is checked only for `app/restart` and `app/shutdown` (`src/network_app.rs:86-87`). Engine routes ignore the method, so a "GET torrents/delete" works.
- **Concurrency and timeouts:** ≤16 concurrent workers; the 17th client gets "busy" (`src/ipc.rs:207-217`). Server read timeout 10 s. The client uses `timeout_secs` (default 10), or ≥95 s for `torrents/add` (`src/ipc.rs:96-100`). Each worker blocks in `runtime.block_on` on a one-worker runtime.

| Route | Args | Result |
|---|---|---|
| `app/version` | – | version string (liveness probe used by `is_running`) |
| `app/network` | – | `NetworkState` JSON (VPN selection, readiness) |
| `app/settings` | – | `{max_active_downloads, peer_encryption, network_interface}` |
| `app/restart` (POST) | – | sets RESTART; TUI tick quits and `run_native_app` reopens the engine |
| `app/shutdown` (POST) | – | sets CLOSE/QUIT; TUI tick quits |
| `app/queue` | – | `{max_active_downloads, active_downloads, queued_downloads, network_blocked}` |
| `app/setQueue` | `max_active_downloads` | persists config and re-runs the queue |
| `app/setPreferences` | – | always an error (native) |
| `transfer/info` | – | `{dl_info_speed, up_info_speed, dl_info_data, up_info_data, connection_status}` |
| `torrents/info` | – | `[Torrent]` (qBittorrent-style states plus `queuedDL`/`waitingVPN`) |
| `torrents/files` | `hash` | `[TorrentFile]` (priority 0/1) |
| `torrents/add` | `urls` (newline-separated magnets, URLs or local paths) or `file`; `savepath`, `category`, `tags`, `upLimit`, `ratioLimit`, `stopped`/`paused` | `"Ok."` (no hash returned; the CLI diffs `torrents/info` before and after, `src/main.rs:525-532`, `550-556`, which races with concurrent adds) |
| `torrents/start`, `torrents/stop` | `hashes` (`a|b` or `all`) | durable pause intent and queue reconcile; start on an errored torrent rebuilds and rechecks it |
| `torrents/delete` | `hashes`, `deleteFiles` | the record is removed durably before payload deletion |
| `torrents/filePrio` | `hash`, `id` (`1|2`), `priority` 0/1 | include/skip |
| `torrents/setUploadLimit` | `hashes`, `limit` (0 or ≥16384) | rebuilds the torrent (re-check) |
| `torrents/setShareLimits` | `hashes`, `ratioLimit` −2 or 0 with `shareLimitAction=Stop` | – |
| `torrents/addTags`, `torrents/removeTags` | `hashes`, `tags` | – |
| `torrents/toggleSequentialDownload`, `torrents/toggleFirstLastPiecePrio` | – | always an error (intrinsic) |

The **parked (VPN-waiting) server** answers a subset from the on-disk ledger: version, shutdown, queue, setQueue, transfer/info, info, files, start, stop, and delete-without-files (`src/engine.rs:1233-1352`). Every other `torrents/*` route returns an error.

**Client side** (`qbt::Client`, `src/qbt.rs:249-630`): a concrete struct, not a trait. It holds two transports behind per-method `if self.cfg.native()` branches. RSS and search calls in native mode run **in the calling process** (`src/qbt.rs:548-627` call `crate::rss::*` and `crate::search::*`), not in the engine. That is why `fbittorrent search` works with the app closed, and why search jobs die with the TUI.

## 8. qBittorrent feature inventory (native mode)

| Feature | fbittorrent | Evidence |
|---|---|---|
| Add magnet / .torrent / URL | Yes | `src/engine.rs:826-978` (urls, local path at 900, uploaded file at 883-899); CLI `add` `src/main.rs:502-556`; TUI `a`/`c` `src/tui/mod.rs:1220-1228` |
| File selection + priorities | Partial (include/skip only) | `src/engine.rs:653-703`; high/max refused at 657 |
| Sequential / first-last | Yes (always on, not toggleable) | `vendor/rqbit/librqbit/src/file_info.rs:14-25`; `src/engine.rs:648-649` |
| Pause / resume / force-start | Partial (no force-start, no queue bypass) | `src/engine.rs:753-765` |
| Recheck | No user action (implicit on every open; on start-after-error) | `src/engine.rs:437`, `756-760` |
| Reannounce | No | no route (`src/engine.rs:550-571`) |
| Rename (torrent/file) | No | – |
| Move / set location | No (set only at add: `--save`/`savepath`) | `src/main.rs:513`, `src/engine.rs:858-861` |
| Categories | Partial (stored and passed through; no UI or filter) | `src/engine.rs:873`; `src/main.rs:514` |
| Tags | Partial (only the internal `seed` tag is used) | `src/engine.rs:788-810`; `src/qbt.rs:503-518` |
| Per-torrent speed limits | Partial (upload cap only; changing it rebuilds/re-checks; no download cap) | `src/engine.rs:66-85`, `766-776` |
| Global speed limits | No | `SessionOptions` has no limits (`src/engine.rs:417-439`) |
| Alternative limits / scheduler | No | – |
| Queueing (max active) | Yes (downloads only; FIFO by add order; no reorder) | `src/engine.rs:280-343`, F6 `src/tui/mod.rs:1082-1137` |
| Share ratio / seed time limits | Partial (unlimited or stop at completion only) | `src/engine.rs:777-787` |
| Super seeding | No | – |
| Peers view | No (peer count only, mislabeled) | `src/engine.rs:1405-1415`, `1479-1481` |
| Trackers view (add/edit/remove) | No | – |
| Piece map / progress bar | Partial (progress bar only) | `src/tui/mod.rs:2960-2962` |
| Speed graphs | No (current rates in the tab bar) | `src/tui/mod.rs:1996-2003` |
| Statistics (session totals) | Partial (served by `transfer/info` but never displayed) | `src/engine.rs:616-623` |
| Execution log | No (status line expires after 8/15 s; engine errors go to `last-error.txt`) | `src/tui/mod.rs:1705-1706`; `src/engine.rs:484-487` |
| RSS feeds | Yes (local, durable read state) | `src/rss.rs:61-290`; TUI `src/tui/mod.rs:1469-1528` |
| RSS auto-download rules | No (by design: "checking never downloads") | README L37 |
| Search plugins | Yes (Nova/Python, 18 installed, install from URL) | `src/search.rs:256-340`, `518`, `738-843` |
| Torrent creator | No (library code unexposed) | `vendor/rqbit/librqbit/src/create_torrent_file.rs` |
| IP filter | No | – |
| Encryption | Yes (off/prefer/require) | `src/engine.rs:418-422` |
| Proxy (SOCKS/HTTP) | No | EE "SOCKS … outside this integration" |
| VPN / interface binding | Yes (exact device, waiting state) | `src/network_app.rs:24-36`, `92-106`; `src/engine.rs:425` |
| DHT / PeX / LSD toggles | No (always on; only tests disable them) | `src/engine.rs:417-439` vs `1523-1525` |
| Port settings / UPnP | Partial (`peer_port` in config, not in UI; UPnP forced off; TCP only, no uTP) | `src/engine.rs:426-430` |
| Remote backend | No in native (local socket only); legacy WebUI URL only | `src/ipc.rs:58-60`; `src/qbt.rs:258-271` |
| Notifications on completion | No (only "added"/"add failed") | `src/main.rs:527-623` |
| Magnet / .torrent handler | Yes | `src/setup.rs:105-123`; `xdg-mime` → `fbittorrent-magnet.desktop` |
| Theming | Yes (Omarchy colors, live reload) | `src/tui/theme.rs:57-79`; `src/tui/mod.rs:779-794` |

**Score: 9 Yes, 9 Partial, 18 No (of 36).** Beyond qBittorrent, the app adds a Shows watchlist with an unread inbox, IMDb/Wikipedia cards, a provider browser, VPN wait-and-restore, and an agent-ergonomic CLI.

## 9. UX audit (TUI + bar widget)

**Look and feel, honestly:** calm, monochrome, tidy box-drawing (rounded borders, right-aligned numeric columns). It reads as a careful engineering prototype rather than a polished app.

- **Information density is poor where it matters.** The torrents screen spends a 45-row pane on one row. Below 140 columns (the user's real tiled window is about 90) the details pane disappears entirely, along with state, peers, path and the IMDb card (`screens/tui-real.png`, `screens/21-torrents-80x24.txt`).
- **Hierarchy** comes from tabs, pane titles and bold headers, but the headers and every dimmed label use `#505050` on black.
- **Color tokens:** only fg/dim/accent/selection/red/green/yellow are used. blue/cyan/magenta are loaded but unused (`src/tui/theme.rs:71-73`).
- **Empty states** are good on Search ("Type a query, then (Enter) Search") and in the widget ("Nothing here yet. Copy a magnet link and press a.").
- **Help** is thorough but 68 lines in a fixed 84×34 box, three pages (`screens/03-*`).
- **Mouse support** is real (verified).
- **Consistency across the four screens:** layout yes, key meanings no.

**Heuristic scores:**

| Heuristic | Score |
|---|---|
| 1 Visibility | 5/10 |
| 2 Real world | 6/10 |
| 3 Control | 5/10 |
| 4 Consistency | 4/10 |
| 5 Error prevention | 4/10 |
| 6 Recognition | 6/10 |
| 7 Flexibility | 6/10 |
| 8 Minimalism | 5/10 |
| 9 Error recovery | 5/10 |
| 10 Help | 7/10 |
| **Overall** | **5.3/10** |

**Top 15 UX defects**

| # | Sev | Heuristic | Defect | Evidence |
|---|---|---|---|---|
| 1 | High | 1, a11y (not color-only) | Torrent state, peers, path and IMDb card are invisible below 140 columns. State is conveyed only by name color or dimming, with no text column, and there is no key to show details. | `screens/tui-real.png` (about 90 columns, real window); `screens/21-torrents-80x24.txt`; `src/tui/mod.rs:2774-2776`, `2963-2964` |
| 2 | High | 1, 9, a11y | Semantic colors collapse on the active Omarchy theme: red `#a4a4a4` vs green `#b6b6b6`. Success vs error feedback, seed/rating coloring and error rows are indistinguishable. | `~/.local/state/omarchy/current/theme/colors.toml`; `src/tui/mod.rs:2132`, `3005-3009`, `2964` |
| 3 | High | a11y contrast | `dimmed()` uses `dark_foreground` `#505050` on `#000` (≈2.6:1). Used for column headers, footer action labels, inactive tabs and hints. | `src/tui/theme.rs:66-67`; ANSI `screens/01-torrents-160x45.ansi.txt` line 4 (`38;2;80;80;80`) |
| 4 | High | 1, 8 | At 80 columns, RSS item names lose the episode id ("Slow Horses S0…sta EZTV"). Middle ellipsis keeps the release group, and an always-"0 B" size column wastes 9 cells. | `screens/20-rss-80x24.txt` L4-19; `screens/14-rss-feeds.txt` |
| 5 | High | 5 | The destructive confirm defaults to yes. Enter confirms "DELETE '…' AND ITS FILES? y/n"; any other key cancels. | `src/tui/mod.rs:1260`, `1346-1348`, footer `2023` |
| 6 | High | 3, 5 | Quitting stops every transfer without warning: `q`/`Q`/Ctrl+C in the TUI, and `q` or the power icon in the widget. No "N downloads active" prompt; no detach option. | `src/tui/mod.rs:1178`, `1207-1212`; `plugin/oliver.torrents/Panel.qml:179`, `226` |
| 7 | Medium | 1, 9 | `(P) Resume` on a finished stop-on-complete torrent reports "started" and does nothing. | `src/tui/mod.rs:2040`, `1325-1328`; `src/engine.rs:306-310`; `screens/tui-real.png` |
| 8 | Medium | 4 | The SPEED column shows total size for inactive torrents. | `src/tui/mod.rs:2954` vs header `3030`; `screens/01` row "100.0% 3.4 GiB" |
| 9 | Medium | 2 | Misleading counts: "0 seeds, N leechers" (native never knows seeds), widget "0s", YTS rows "0" seeds and RSS "0 B" for unknown values. | `src/engine.rs:1479-1481`; `src/tui/mod.rs:2199`; `Model.js:79`; `screens/10`, `screens/14` |
| 10 | Medium | 8, a11y (motion) | The selected row's name marquee-scrolls continuously and wraps ("he Diplomat…", "ESub   The Diplo"), so the title start is usually hidden. | `screens/01-torrents-160x45.txt` L5, `screens/21`, `screens/24-torrents-60x20.txt` |
| 11 | Medium | 6 | The footer drops actions silently at narrow widths: at 80×24, Torrents loses `(Shift+D)`, `(A)dd`, `(C)lipboard` and `(F6)` with no overflow cue. | `screens/21-torrents-80x24.txt`; `src/tui/mod.rs:2119-2124` |
| 12 | Medium | 4 | Key meanings change by screen: `u` seed-toggle vs undo; `n` new feed / follow / none; `a` add vs all; `i` open browser vs inline lookup on RSS; `x` vs `d` for removal; `f` vs `F` follow. The footer prints uppercase letters for lowercase keys, so Help has to explain it. | `src/tui/mod.rs:1337` vs `1574`; `1483`/`1560`/`1394`; `1220`/`1393`; `1353` vs `1515`; `2657` |
| 13 | Medium | 1, 9 | Feedback is ephemeral and unlogged. The status line clears after 8 s (15 s for errors), there is no message history, and background engine errors go only to `engine/last-error.txt`. | `src/tui/mod.rs:1705-1706`; `src/engine.rs:484-487` |
| 14 | Medium | 8, 2 | Search result quality: duplicates are visible and provider mojibake is shown raw. UPLOADED/SOURCE/PEERS are hidden whenever the details pane is up, even at 160 columns. In browse mode the query box still says "Sintel" over a YTS list. | `screens/07-search-sintel-16s.txt`, `screens/22-search-80x24.txt`; `src/tui/mod.rs:2979-2997`, `3000`, `3023` |
| 15 | Medium | 3, 5 | Widget: the trash button and `d` remove a torrent immediately, with no confirm and no undo. The widget is hidden while the app is closed, so the bar offers no way to open the app (`visible: svc.online`). | `Service.qml:62-65`; `Panel.qml:31`, `181`, `450-459` |

Lower-severity notes:
- Browse opens on a browser-only, Cloudflare-blocked 1337x entry (`src/browse.rs:34-36`).
- The files modal lists files in metainfo order, so E07 comes before E01 (`screens/02`).
- F6 exposes only 3 settings. Save path, port, seeding default, show quality and limits are config-file only (`screens/04`).
- Lists have no PgUp/PgDn, and RSS has no Home/End.
- The Shows IMDb card is squeezed into 9 rows while the right pane is mostly empty (`screens/12`).
- "(F)ollow show" is offered on films (`screens/08`).
- The desktop entry borrows the `qbittorrent` icon (`src/setup.rs:112`).
- `shows.json` is world-readable (0644) while the other stores are 0600.

## 10. Code-health notes

- **`src/tui/mod.rs` (3504 lines; 3034 code + 470 tests).** One `TuiApp` holds about 70 fields covering 4 screens, 3 modals, mouse geometry, IMDb cache, kitty placement, RSS revision tokens and Show undo (`src/tui/mod.rs:141-214`). Key handling (1160-1646), views (1975-2746) and helpers sit in one `impl`.
  - Footer hints, Help text and handlers are three hand-maintained copies of the key map (`2037-2104`, `2642-2721`, `1160-1646`). Drift is already visible (Help says "letters use lowercase" because the footer shows uppercase).
  - Mouse hit-testing re-derives the layout separately from rendering (`mouse_list_at`, `796-843`), as the project's own UX review notes.
  - Natural seams: `screens/{torrents,search,shows,rss}.rs`, `modals.rs`, `footer.rs` (one action table generating footer, help and handlers), `mouse.rs` (using rects recorded during render).
- **`src/engine.rs` (2694 lines; ~1500 code + ~1200 tests).** It mixes four concerns:
  - ledger persistence (`save`, `write_metadata`);
  - queue policy (`reconcile_queue`);
  - string-route IPC dispatch;
  - legacy qBittorrent import and the parked VPN server.

  Route names are matched in four places (`550-571`, `587-651`, `1234-1246`, `src/network_app.rs:78-90`), so a new route must be added in sync by hand. `snapshot()` translates rqbit states into qBittorrent state strings that the TUI and widget then re-interpret (`src/tui/mod.rs:2933-2949`, `Model.js:40-57`). That is three copies of the state vocabulary.
- **Process-global lifecycle statics:**
  - `QUIT_REQUESTED` (`src/engine.rs:32`);
  - `RESTART`, `CLOSE` and `EXPECTED` (`src/network_app.rs:9-11`).

  They are polled by the TUI, which works for one engine per process but blocks embedding or testing two engines in one process.
- **Legacy tax:** `qbt::Client` branches native/legacy on every method, and the TUI keeps legacy-only keys (`s`/`f`, `2`/`3` priorities). Legacy mode is no longer the user's mode; its tests (`qbt_tests`, 11) still pass.
- **Per-frame syscalls:** `view_torrents` calls `network_app::state()` on every render (`src/tui/mod.rs:2148`), which queries the kernel interface when binding is configured. `maybe_reload_theme` stats a file every 2 s.
- **Robustness:**
  - any single record failure aborts app start (`src/engine.rs:458-463`);
  - full re-hash on every open (`src/engine.rs:437`);
  - `expect("TODO")` panics in vendored rqbit (`session.rs:1310`, `torrent_state/mod.rs:449`);
  - non-graceful terminal close (§5.2).
- **Positive:**
  - no TODO/stub debt in app code;
  - durable fsync'd writes everywhere (ledger, shows, rss, config);
  - bounded subprocess capture with process-group kill and `PR_SET_PDEATHSIG` for providers (`src/search.rs:583-688`);
  - single-instance lock;
  - a small, auditable IPC server with bounded workers.
- **Effort skew:** 11–14 October went almost entirely into download-CPU qualification (ten "gauntlet" rounds, `docs/validation/2026-10-13/per-chunk-cpu/` = 231 MB, an 18k-line `oracle-source-context.md`), while §8's user-visible gaps were untouched and nothing was committed. The receipts are rigorous, but they are a poor trade against shipping.

## 11. Test results (evidence files)

| Run | Result | File |
|---|---|---|
| `cargo build --release` (Rust 1.98.0) | OK, 8m21s, 5 `librqbit` warnings | `evidence/cargo-build-release.txt` |
| `cargo test --release` (README command; default member `fbittorrent` only) | **90 passed, 0 failed, 0 ignored** (6.81 s test time, 1m19s wall) | `evidence/cargo-test.txt` |
| User state before/after `cargo test` | config, shows, rss, search and torrents ledger hashes unchanged | (checked; hashes not stored) |
| `native.py target/release/fbittorrent` (isolated XDG, real tmux TUI, local fixtures) | **all 11 steps ok, "native integration passed"** | `evidence/native-py.txt` |
| Engine libraries, combined command from `docs/validation/2026-10-10/README.md` | **Build failure** (E0428/E0080: `sha1-crypto-hash` and `sha1-ring` both enabled) | `evidence/cargo-test-engine-libs-combined-FAILED-BUILD.txt` |
| Engine libraries, split commands from `docs/engine-evaluation.md` | dht+tracker+dualstack **39 passed**. `rqbit-runtime` **14 passed** (1 ignored). The EE command `-p librqbit-core -p librqbit` **also fails to build** (same sha1 conflict, EXIT 101). Run separately: `librqbit --features rust-tls` **97 passed** (3 ignored: libtorrent-oracle/namespace fixtures) and `librqbit-core --no-default-features --features sha1-ring` **32 passed**. **Total 182 passed, 0 failed, 4 ignored.** | `evidence/cargo-test-engine-libs.txt` |
| Widget autolaunch probe | `status` never launches; `reconcile`/`list` launch the app | `evidence/reconcile-autolaunch-probe.txt` |
| App lifecycle (quit vs window close) | `q` clean; window close leaves a stale socket | `evidence/app-lifecycle-observations.txt` |
| Not run | `run.sh`, `tui_smoke.sh` (need a startable qbittorrent-nox: legacy), `widget_keys.sh` (adds two torrents to the live engine), `vpn.py`, `resources.py`, benchmarks | – |

## 12. Open risks

1. **Total-loss risk:** the whole native product is uncommitted. Commit it in reviewable slices:
   - `vendor/` as its own commit with provenance;
   - engine and IPC;
   - shows/rss/search;
   - TUI;
   - widget.

   Move or prune `docs/validation/` (279 MB) out of the repo, or into LFS, before committing.
2. **Widget resurrection bug** (§5.1). Users will see the app reopen after quitting.
3. **Engine CPU** is still about 3.2× qBittorrent. Startup re-hash scales with the library size (unmeasured), and every open re-verifies everything.
4. **Single-process coupling:** no headless mode. Any crash or terminal close kills transfers, and a second GUI/dashboard cannot exist without the TUI.
5. **Unproven in the real world:**
   - WAN swarm throughput and private trackers;
   - real ProtonVPN `proton0` loss and recovery (only namespace fixtures);
   - large libraries;
   - uTP-only peers (unsupported);
   - BitTorrent v2 (refused).
6. **Vendored forks:** Asupersync 0.5.0 (patched timer wheel) and rqbit 9.0.1 (runtime port). There is no upstream sync plan, so security fixes in rustls, webpki and similar dependencies must be tracked manually.
7. **Contract drift:**
   - the agent docs state the wrong upload cap;
   - README claims are stale (YTS, UPLOADED column, dedupe);
   - both documented multi-crate library-test commands no longer build.
8. **Accessibility on monochrome themes:** semantics depend on hue, and the user's current theme removes it.
