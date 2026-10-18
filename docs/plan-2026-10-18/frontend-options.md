# fbittorrent: frontend and architecture options

Research date: 2026-10-18. This is input for the planning pass, not the plan.
It answers one question from the owner: should fbittorrent move from its
FrankenTUI to a Quickshell-native UI like OmaTorrent's, or offer both, over an
fbittorrent backend?

Scope: the frontend and process-topology option space and its feasibility.
A separate review audits fbittorrent's code, and another researches OmaTorrent
itself. Facts about fbittorrent cite `file:line` in the working tree as of today
(branch `codex/shows-native-engine`, uncommitted diff included). External facts
cite a version, commit or URL. Clones used for this research are read-only and
were never built or run; they live under the session scratchpad
(`scratchpad/qsr/`, `scratchpad/prior-art/`).

---

## 0. Recommendation in one page

**Recommendation: offer both, over a headless engine daemon. Make the
Quickshell UI the main graphical surface, keep the TUI as the terminal and SSH
surface, and do not add a Rust GUI toolkit.**

Topology:

- **One owner of torrent state.** The same `fbittorrent` binary gets a
  `fbittorrent serve` mode, run as a systemd *user* service that is never
  enabled. Clients start it on demand. "Nothing runs at login" still holds.
- **Clients:**
  - the Omarchy plugin, as a bar popup plus a full dashboard window;
  - the FrankenTUI;
  - the CLI and `magnet:` handler.
- **API:** one versioned API (NDJSON over the existing private Unix socket
  directory), with a snapshot followed by pushed deltas, replacing per-tick
  polling.

Why:

1. **A Quickshell UI is impossible to do well while the engine lives inside the
   TUI process.** Today, opening any "app" means opening a terminal, because
   `run_native_app` starts the engine, then blocks in `tui::run`, then drops the
   engine (`src/main.rs:786-826`). A dashboard that pops a Ghostty window to host
   its engine is the worst of both worlds. The daemon split is the enabling
   step for every graphical option, so it comes first whatever frontend wins.
2. **The split is cheap because the TUI is already an IPC client of its own
   engine.**
   - It polls `torrents/info` and `transfer/info` through `Client` →
     `ipc::request` every 2 s (`src/tui/mod.rs:353-361`, `:1672`).
   - The only in-process couplings are three global flags it reads on each tick:
     `engine::quit_requested`, `network_app::close_requested` and
     `check_transition` (`src/tui/mod.rs:1700-1701`).
3. **Quickshell is the only option that can look exactly like Omarchy.**
   - It uses the same `Panel`/`KeyboardPanel` chrome, `qs.Commons` color and
     style tokens, and keyboard model as the Wi-Fi, Bluetooth and Tailscale
     panels.
   - The theme switches live, pushed by `omarchy-theme-set` over shell IPC
     (`/usr/share/omarchy/bin/omarchy-theme-set:219`).
   - A plugin may open a real window. First-party `omarchy.dev-gallery` is a
     `panel` plugin owning a `FloatingWindow`
     (`/usr/share/omarchy/shell/plugins/dev-gallery/GalleryPanel.qml:278`).
     The marketplace plugin `aweiward.omaqbt` is a full qBittorrent client built
     the same way: a 1600×900 FloatingWindow with search, RSS and settings
     (`omaqbt Client.qml:695-701`).
   - No Rust toolkit gets the Omarchy palette for free. Omarchy only sets
     Adwaita/Adwaita-dark and the color scheme for GTK, and gives Qt the gtk3
     platform theme (§3.1).
4. **The TUI is worth keeping, but it should become a pure client.**
   - It works over SSH and on a TTY.
   - It carries all the keyboard muscle memory and has tmux-driven regression
     tests (`agent_ergonomics_audit/audit/regression_tests/tui_smoke.sh`).
   - It is also the fallback when the shell is restarting or broken.
   - Once both UIs are thin clients of one daemon, the cost of having both is
     the UI code only, not two engines or two sets of semantics.

Main risks (detail in §7.3):

- **No isolation inside the shell.** QML runs inside the single
  `omarchy-shell` process, which also hosts the bar, notifications, polkit and
  the lock screen. There is no sandbox, and a stall or crash there affects the
  whole desktop.
- **Platform churn.** Quickshell is pre-1.0; Omarchy had to patch 0.3.2 for
  Qt 6.12. Omarchy 4 still reports `4.0.0.alpha` in its version file.
- **Lifetime semantics change.** Closing a window no longer has to stop
  downloads, so the owner must choose the stop policy (§7.4).
- **Two UIs can drift in features** unless every feature lands in the daemon
  API first.

The scores in §6 put "daemon + Quickshell + TUI" (78/95) and "daemon +
Quickshell only" (77/95) in a tie. The plan should take the first as the
migration path. Retiring the TUI later is a separate decision, made once the
dashboard reaches parity and real use shows whether the TUI still earns its
upkeep.

---

## 1. Where fbittorrent is today (task C, facts)

### 1.1 Process and lifetime

- **The engine belongs to the TUI process.** The sequence in
  `src/main.rs:786-826` is:
  1. `run_native_app` checks `ipc::is_running` and, if the engine already runs,
     focuses the existing window (`:787`).
  2. Otherwise it computes the VPN snapshot.
  3. Then either it starts a **parked** engine (VPN not ready; `:790-800`), or
     `engine::start` followed by `app.serve(handler)` (`:804-816`).
  4. It runs `tui::run` (`:818`), then drops the engine (`:819`).
  5. It loops on a restart request, after a settings change or VPN transition
     (`:821-824`).
- **`fbittorrent tui` refuses to run without a TTY** (`src/main.rs:452-455`).
  There is no headless mode.
- **Single instance:** `InstanceLock` takes `flock(LOCK_EX|LOCK_NB)` on
  `$XDG_RUNTIME_DIR/fbittorrent/engine.lock` (directory 0700, file 0600;
  `src/ipc.rs:130-163`).
- **The engine itself:**
  - It runs on its own Asupersync runtime with 2 worker threads, a peer limit of
    96, TCP-only listen and no UPnP (`src/engine.rs:395-440`).
  - It has a 1 s `maintain` loop for queue enforcement; errors go to
    `last-error.txt` (`src/engine.rs:471-491`).
  - Drop drains the session within 4 s, then shuts the runtime down
    (`src/engine.rs:517-546`).
- **The README states the user-facing contract:**
  - "CLI and widget actions open or focus the app when they need its engine.
    Status queries never open it. Nothing runs at login. Closing the terminal or
    quitting the TUI stops the native engine" (`README.md:17-20`).
  - Project memory records the same owner decision of 2026-09-03 for the
    earlier qBittorrent backend: on demand, not at login, stopped by `down`, the
    widget power button or `Q`.

### 1.2 The IPC as implemented

| Aspect | Today | Source |
|---|---|---|
| Transport | Unix stream socket `$XDG_RUNTIME_DIR/fbittorrent/engine.sock`, mode 0600 | `src/ipc.rs:58-60,186-187` |
| Framing | 4-byte big-endian length + JSON, max 32 MiB | `src/ipc.rs:26,62-85` |
| Exchange | One request per connection, then close; 10 s read timeout | `src/ipc.rs:197-242` |
| Request | `{method, path, args: [(k,v)], file: {name, bytes}?}`. Paths mimic the qBittorrent WebUI v2 (`torrents/add`, `app/version`) | `src/ipc.rs:35-50` |
| Response | `{value: String?, error: String?}`. JSON payloads are double-encoded as strings | `src/ipc.rs:52-56,117-121` |
| Concurrency | At most 16 clients; one OS thread per connection, reaped as they finish | `src/ipc.rs:27,207-253` |
| Versioning | None. `app/version` returns `"native librqbit <v>"` | `src/engine.rs:588` |
| Push / events | None; every client polls | n/a |

**Routes the engine serves** (`src/engine.rs:549-571`):

- `app/version`, `app/shutdown`, `app/queue`, `app/setQueue`.
- `app/setPreferences`, which is rejected.
- `transfer/info`.
- `torrents/` `info`, `add`, `start`, `stop`, `delete`, `files`, `filePrio`,
  `setUploadLimit`, `setShareLimits`, `addTags`, `removeTags`.
- `torrents/toggleSequentialDownload` and `toggleFirstLastPiecePrio`, both
  rejected because these policies are always on.

**App controls** (`src/network_app.rs:78-90`): `app/network`, `app/settings`,
`app/restart` and `app/shutdown`.

**Parked engine.** While the VPN is not ready, `dispatch_parked` answers status
and records from the ledger and refuses actions (`src/engine.rs:1233-1355`,
`src/network_app.rs:98-105`).

**What librqbit has but the IPC does not expose:** per-peer stats
(`vendor/rqbit/librqbit/src/torrent_state/live/mod.rs:782`), the haves bitfield,
and trackers. The fork compiles out librqbit's own HTTP API with `#[cfg(any())]`
because that API is axum/tokio (`vendor/rqbit/librqbit/src/lib.rs:36-41`). A
richer UI needs new routes for peers, pieces and trackers.

### 1.3 What goes through the socket and what does not

| Feature | Where it runs in native mode | State |
|---|---|---|
| Torrents, queue, transfer stats | Engine, over IPC | `~/.local/share/fbittorrent/engine/torrents.json` ledger + saved metadata (`src/engine.rs:398-401`) |
| RSS | **In the calling process.** `Client::rss_*` calls `crate::rss` directly (`src/qbt.rs:546-585`) | `~/.local/share/fbittorrent/rss.json`, guarded by a flock'd `.lock` and atomic rename (`src/rss.rs:69-71,95-156`) |
| Search providers | **In the calling process.** `Client::search_*` calls `crate::search`, which runs nova3 Python runners per process (`src/qbt.rs:587-628`, `src/search.rs:1-3,29-47`) | provider preferences under `data_dir` (`src/search.rs:68-78`) |
| Shows | In the calling process (`src/main.rs:1022+`) | `~/.local/share/fbittorrent/shows.json` (`src/shows.rs:208-213`) |
| IMDb cards | In the calling process | `~/.cache/fbittorrent/imdb` |
| Config | Read by each process | `~/.config/fbittorrent/config.toml` (`src/config.rs:130-146`) |
| VPN binding | Engine sockets only (`bind_device_name`, `src/engine.rs:421-423`). Search/RSS/IMDb use the normal route by design; provider download helpers are refused when binding is on (`src/qbt.rs:413-423`, `README.md:79-86`) | `network_interface` in config |

**In-process only:** the librqbit session (peers, rates, piece state), the
network snapshot and parked state, running search jobs, the TUI's IMDb ratings
index and view state.

### 1.4 How the CLI, widget and magnet handler reach the app

- **The CLI** calls `backend::ensure_running` (`src/backend.rs:17-30`). If the
  socket does not answer and `autostart` is on, `open_app()` spawns
  `omarchy-launch-or-focus-tui --app-id=org.omarchy.fbittorrent '<exe>' tui`
  (`src/backend.rs:62-86`), then polls up to 60 s for the socket.
  - So a `magnet:` click from the browser (`fbittorrent add`) **opens a terminal
    window** as a side effect.
  - `status` never starts anything (`src/main.rs:61`, `:407-415`).
  - `down` posts `app/shutdown` and waits for the instance lock to free
    (`src/backend.rs:44-59`).
- **The widget:**
  - `plugin/oliver.torrents/Service.qml` spawns `fbittorrent status --json`
    every 2 s while open and every 10 s while closed (`Service.qml:80-118`).
    Each run is three IPC connections (version, transfer info, torrents;
    `src/main.rs:407-430`).
  - Actions run as serialized CLI processes (`Service.qml:44-78`), plus a
    `reconcile` every 60 s (`:120-126`).
  - The widget hides itself while the app is closed (`Panel.qml:31`).
  - Measured today: one `status --json` takes 4–5 ms wall time.
  - **Per-monitor polling.** The `Service` is instantiated inside the bar
    widget (`Panel.qml:119-122`), and Omarchy builds a widget once per monitor.
    Omarchy's shell guide explicitly warns against exactly this ("a Timer that
    starts a Process costs that process times the number of monitors";
    `agents/skills/shell-dev.md:42-51` @ omarchy `4e059ac`).

### 1.5 A headless mode does not exist; what one would take

| Work item | Size | Notes |
|---|---|---|
| `fbittorrent serve` (hidden subcommand) | S | Same body as `run_native_app`, with `tui::run` replaced by a wait loop on the three flags the TUI tick checks today (`src/tui/mod.rs:1700-1701`). Restart-in-place on settings change or VPN transition, as now (`src/main.rs:821-824`). SIGTERM sets `QUIT_REQUESTED` so `Drop` drains (`src/engine.rs:517-546`). |
| systemd user unit, installed by `setup`, never enabled | S | `ExecStart=%h/.local/bin/fbittorrent serve`, `Restart=on-failure`, `TimeoutStopSec=15`. `Type=notify` is optional (Transmission reports speeds in `STATUS=`; prior art §5). Logs go to journald instead of `last-error.txt`. |
| `ensure_running` → `systemctl --user start fbittorrent` | S | Removes the terminal side effect and the 60 s poll race. `open_app()` stays only for "open the TUI". Legacy code already drives a user unit this way (`src/backend.rs:33-41`). |
| TUI becomes a client | S | Start the daemon if absent, then run as today. Split "close UI" from "quit engine". |
| Notifications, sleep inhibitor, VPN watch inside the daemon | S–M | Today completion notifications ride on CLI `add`. The VPN watch is the TUI tick. |
| Upgrade handling | S | A new binary talking to an old running daemon must detect the version mismatch (§7.2). |
| Move RSS/shows scheduling and search jobs into the daemon | M | Needed so the QML UI does not run Rust code per action, and so feeds refresh while no TUI is open. |

---

## 2. Omarchy shell and Quickshell as an app platform (task A)

### 2.1 Versions

| Component | Version on this machine | Upstream |
|---|---|---|
| Omarchy | `omarchy-dev 4.0.0.r7025.g8a787b6` (pacman); `/usr/share/omarchy/version` = `4.0.0.alpha` | basecamp/omarchy default branch `quattro` @ `4e059ac` (2026-10-18); latest release v4.0.4 (2026-09-15) |
| Quickshell | `quickshell 0.3.2-6` | tag v0.3.2 `4f508be` (2026-10-08); canonical repo git.outfoxxed.me/quickshell/quickshell, mirror github.com/quickshell-mirror/quickshell |
| Qt | qt6-base 6.12.0, qt6-declarative 6.12.0 | Quickshell needs Qt ≥ 6.6 and **must be rebuilt for every Qt release** because it uses private APIs (`BUILD.md:42-43,59`) |

- **The local Quickshell package is patched.** Omarchy builds it with
  `load_qtquick_611.patch` as a Qt 6.12 workaround (omacom/omarchy-pkgs #935,
  2026-10-10). The patch is enabled by `//@ pragma OmarchyLoadPatch` on line 1
  of `/usr/share/omarchy/shell/shell.qml`.
- **Changes in Quickshell 0.3.0 that matter here:**
  - FloatingWindow gained `minimized`, `maximized` and `fullscreen`, plus
    move/resize and parent (dialog) windows.
  - `AppId` pragma.
  - IPC signal listeners.
- **0.3.2** added Qt 6.12 support and fixed `Socket` reconnects.

### 2.2 The plugin model

- **One process.** "The Omarchy desktop runs in one long-lived Quickshell
  process. Its bar, panels, overlays, menus, and services are plugins hosted by
  `shell.qml`" (`/usr/share/omarchy/shell/README.md`). The running process
  measured about 300–350 MB RSS after two days.
- **Kinds** (`docs/omarchy-shell.md:37-50` @ `4e059ac`):
  - `bar-widget`, `bar`, `panel` ("Floating window"), `overlay` ("Fullscreen
    overlay"), `menu`, `service` ("Headless singleton").
  - There is no "app" or "dashboard" kind. A window is a `panel` that owns a
    `FloatingWindow`.
- **Manifest:**
  - Required: `schemaVersion: 1`, `id`, `name`, `version`, `kinds`,
    `entryPoints` (`shell/services/PluginRegistry.qml:48-89`).
  - Entry points must stay inside the plugin directory (`:118-130`).
  - Optional: `barWidget{displayName, category, defaultSection, defaults,
    schema}` and `keepLoaded`.
  - `omarchy plugin validate` mirrors these checks and also rejects any
    symlink inside the plugin folder (`bin/omarchy-plugin-validate`).
- **Lifecycle:**
  - Panels, overlays and menus load on summon and expose
    `open(payloadJson)`/`close()` (`docs/omarchy-shell.md:52`).
  - `keepLoaded` keeps them mounted between summons (`shell.qml:1316-1440`).
  - The host injects a capability-scoped `shell` facade with `summon`, `hide`,
    `toggle`, `isPluginOpen`, `serviceFor` and `updateEntryInline`
    (`shell/services/PluginShellApi.qml:1-70`). The facade is documented as "not
    a QML sandbox: visual plugins share the host object tree" (`:8-9`).
- **Summon routing:** if a plugin is both `bar-widget` and `panel`,
  `omarchy-shell shell summon <id>` goes to the **panel** (the window), not to
  the bar popup (`shell.qml:1158-1196`). omaqbt uses exactly this layout: kinds
  `service`, `bar-widget`, `panel`. The popup keeps its own `ShellIpc` target,
  as fbittorrent's does today (`Panel.qml:124-139`).
- **A `service` kind gives one shared singleton** that panels and widgets reach
  via `service`/`serviceFor` (`shell.qml:929-1070`, `:1407`). This is where one
  socket connection and the torrent model belong.

### 2.3 Surfaces a plugin can open

| Surface | Implementation | Use in Omarchy | Fit for fbittorrent |
|---|---|---|---|
| Bar popup | `KeyboardPanel`: a fullscreen layer-shell `PanelWindow` on the `Overlay` layer with a positioned card. It primes `Exclusive` keyboard focus, then settles on `OnDemand` (`shell/Ui/KeyboardPanel.qml:7-36,96-123`) | Wi-Fi, Bluetooth, Tailscale, Dropbox, Agents, the current `oliver.torrents` | Glance-and-act popup (exists) |
| Overlay | `OverlayWindow`: a fullscreen `PanelWindow`, `WlrLayer.Overlay`, `Exclusive` focus, revealed only once scale and geometry are known (`shell/Ui/OverlayWindow.qml:10-102`) | Menu, emojis, clipboard, image picker, reminders | A quick-add or search palette (type, Enter adds) |
| Window | `FloatingWindow`: a regular xdg toplevel with `title`, `minimumSize`, `maximumSize`, `maximized`, `fullscreen`, `parentWindow` and `startSystemMove`/`Resize` (installed `Quickshell/_Window/quickshell-window.qmltypes:40-137`) | `omarchy.dev-gallery` (`GalleryPanel.qml:278-289`); omaqbt's 1600×900 client | **The dashboard** |

- **Windows have no identity of their own.** All Quickshell windows share class
  `org.quickshell`, because the app id is per process (`src/launch/launch.cpp:81,
  117-118,294` @ v0.3.2). Hyprland rules therefore match on title, as Omarchy
  does for the gallery
  (`/usr/share/omarchy/default/hypr/apps/omarchy-shell.lua:21`). The window
  cannot be a distinct `.desktop` or `magnet:` target. Route those through the
  CLI and `omarchy-shell` IPC.
- **Sizing:** `implicitWidth`/`implicitHeight` only set the initial size
  (`floatingwindow.cpp:62-72`).
- **Focus:** opening a window programmatically does not reliably focus it.
  omaqbt needed an activation workaround (`omaqbt WmFocus.qml:7-17`).

### 2.4 Keyboard model

- `PanelKeyCatcher` (`shell/Ui/PanelKeyCatcher.qml:33-99`) turns keys into
  semantic signals:
  - `j`/`k`/`h`/`l` and arrows → `moveRequested`;
  - Enter/Space → `activateRequested`;
  - `x`/Delete → `deleteRequested`;
  - Esc → `closeRequested`;
  - Tab → `tabRequested` (panel switch);
  - Ctrl+j/k → reorder;
  - any other single character → `textKey(text, modifiers)`.
- It takes keys `BeforeItem`, so text fields must set `blocked` while focused.
- `FilterInput` is a `TextInput`, so IME text (fcitx5) works
  (`shell/Ui/FilterInput.qml:1-20`).
- The current torrent popup already uses this model (`Panel.qml:165-184`).

### 2.5 Theming

- `Commons.Color` reads `current/theme/colors.toml` and `shell.toml` at
  startup. Runtime theme switches are pushed in through shell IPC
  (`shell applyTheme`), and `~/.config/omarchy/shell.toml` is watched live
  (`shell/Commons/Color.qml:7-27,207-257`; `bin/omarchy-theme-set:219-279,516`).
- Surface roles (`popups`, `menu`, …), the control-state alphas and
  `Style.space()` scaling all come from tokens (`shell/Commons/Style.qml:72-256`).
- Plugins that bind to these follow every theme switch instantly. omaproton's
  README makes the same claim for its world map.

### 2.6 Data and process facilities

All from the installed `Quickshell/Io` qmltypes and QS v0.3.2 source.

- **`Process`:**
  - Has argv `command`, `environment`, `stdout`/`stderr` parsers,
    `stdinEnabled`, `write()` and `startDetached()`.
  - Its destructor sends SIGKILL, so a plugin reload kills children
    (`src/io/process.cpp:31-39`).
  - There is no PDEATHSIG. Omarchy had to write a reaper for orphaned
    `inotifywait` children (`PluginRegistry.qml:684-706`).
- **`Socket` and `SocketServer`:**
  - Unix sockets only (QLocalSocket).
  - `write(QString)` sends UTF-8 text, and a `SplitParser` handles framing.
  - There is no automatic reconnect: set `connected = true` again from a
    Timer.
  - **QML cannot practically speak fbittorrent's current binary
    length-prefixed frames.**
  - The Omarchy shell itself serves `omarchy-shell` calls this way, with
    `\x1e`/`\x1f`-separated records (`shell.qml:1649-1679`). That is about
    5 ms per call versus about 45 ms through `qs ipc`
    (`/usr/share/omarchy/bin/omarchy-shell`).
- **Long-lived NDJSON streams are proven:**
  - DankMaterialShell holds request and subscribe sockets to its Go backend
    (`Services/DMSService.qml:94-160`).
  - `omarchy.agents` follows a CLI `--events` stream through `SplitParser`
    (`shell/plugins/agents/Panel.qml:878-900`).
- **HTTP:**
  - `XMLHttpRequest` works but silently drops `Cookie` and `Referer`.
  - There is no WebSocket: QtWebSockets is not installed.
  - There is no image disk cache.
  - Clipboard: `Quickshell.clipboardText` is empty unless a Quickshell window
    has focus (`core/qmlglobal.hpp:138-141`). Use `wl-paste`.
- **Lists:** use `ListModel` with stable keys and `ListView.reuseItems`. In
  0.3.2, `ScriptModel` keyed by `objectProp` does not emit row updates; that is
  fixed only on master (`d7ee6d1`, 2026-10-16).

### 2.7 Hot reload and crash isolation

- **Hot reload:**
  - Omarchy disables Quickshell's own file watcher
    (`QS_DISABLE_FILE_WATCHER=1`, `bin/omarchy-launch-shell`).
  - It runs its own: any change under `~/.config/omarchy/plugins` triggers
    `reloadPlugins()`, which unloads **all** panels, widgets and
    non-`keepLoaded` services, then rescans (`shell.qml:1510-1545`).
  - Plugin state is lost on every save during development, and on shell
    restart. State must live in the daemon.
- **Crash isolation: none.**
  - A native crash or a long JavaScript loop in a plugin affects the bar,
    notifications, polkit and the lock screen.
  - Quickshell's crash handler re-execs the shell, and `omarchy-launch-shell`
    gives up after 5 relaunches in a minute.

### 2.8 Menu and keybindings

- **Menu extensions** live in `~/.config/omarchy/extensions/omarchy-menu.jsonc`.
  Dotted ids form submenus; fields include `action`, `when`, `checked` and
  `aliases`. The file is parsed with live reload (`shell/plugins/README.md`,
  "Omarchy menu").
  - fbittorrent already has `torrents.*` entries.
  - `torrents.stop` is stale: it is gated on
    `systemctl --user is-active qbittorrent-nox`.
- **Keybindings:** panel hotkeys use
  `{ panel = "<plugin id>" }` → `omarchy-shell shell toggle <id>`
  (`default/hypr/helpers.lua:100`, `bindings/utilities.lua:101-108`).
  SUPER+CTRL+T is already "Activity" (btop).

### 2.9 What richer plugins look like

| Plugin | Size | Structure |
|---|---|---|
| `io.github.grichard99.omaproton-vpn` 1.4.2 (installed here) | 4,694 lines (Panel 1,793, Service 1,621, WorldMap, Traffic, three Python helpers) | bar-widget; a `KeyboardPanel` with a world map; drives a CLI and Python |
| `aweiward.omaqbt` 2.1.3 (marketplace) | 5,430 lines (Service 2,075, Panel 1,586, Client 960, TorrentTable 343) | service + bar-widget + panel; FloatingWindow client; Python sidecar over stdin/stdout JSON; vim-style keys, turtle mode, filters; `magnet:` → popup confirm |
| `omarchy.network` (first party) | 2,151 lines | the most complex first-party panel |
| `omarchy.agents` (first party) | Panel + Main + Agent | streams a CLI `--events` process |

**Scale guide.** A Quickshell dashboard at OmaTorrent/omaqbt level is roughly
3–5k lines of QML/JS.

### 2.10 Marketplace rules (omacom/omarchy-plugin-marketplace @ `32a1135`)

- **Submission:**
  - A public GitHub repo with a root `manifest.json`, README, licence and an
    optional preview.
  - Submitted through an issue template.
  - Kinds limited to `bar, bar-widget, menu, overlay, panel, service`.
  - `defaultSection` limited to left, center or right
    (`scripts/build-catalog.mjs:50,801-813`).
- **Security baseline:** a static scan. "Bundled executable binary",
  "installer", "service-management" and "remote-build" are review
  capabilities, not bans (`SECURITY.md:39-92`).
- **Manual setup.** A plugin that needs a native binary gets the
  `manual-setup` label: the one-click install line is replaced by a note
  (`AGENTS.md:148`). fbittorrent would be listed this way; the plugin would ask
  the user to install the binary.
- **Install:** `omarchy plugin add <git-url>` clones, validates and never runs
  plugin code (`manual/32-shell-plugins.md:36-53`).
- **Rules:**
  - "never start a second Quickshell process for a plugin"
    (`site/develop.html:29`);
  - "do not start additional standalone Quickshell instances"
    (`agents/skills/shell-dev.md:5-8`).

### 2.11 Limits that shape the design

1. **Work off the UI thread.** The desktop UI thread is shared, so all heavy
   work (diffing, sorting, JSON volume) belongs in Rust. Push small deltas.
2. **No Process-child daemon.** Do not run the engine as a Process child of the
   plugin; reloads SIGKILL children.
3. **No file picker in Quickshell.** Build a small QML browser, or have the
   daemon call the xdg-desktop-portal FileChooser.
4. **Clipboard through `wl-paste`.** Drag-and-drop onto a FloatingWindow uses
   Qt's `DropArea`. omaqbt claims drops onto its popup work; this research did
   not verify it.
5. **No distinct window identity** (§2.3).
6. **Platform churn.** Quickshell is pre-1.0, with breaking minors and a
   rebuild per Qt release. Noctalia (11.4k stars) has left Quickshell entirely.

---

## 3. Rust GUI toolkits on Omarchy (task B)

### 3.1 How Omarchy themes apps outside the shell

| Consumer | What happens on `omarchy-theme-set` | Source |
|---|---|---|
| Terminals and TUIs | Per-app templates (`ghostty.conf.tpl`, `kitty.conf.tpl`, …) are rendered into `current/theme/` and terminals restart. `omarchy-theme-osc` can recolor a running terminal. fbittorrent's TUI reloads `colors.toml` on mtime change (`src/tui/mod.rs:779-783`) | `/usr/share/omarchy/default/themed/*.tpl`, `bin/omarchy-theme-osc` |
| GTK4 / libadwaita | Only `color-scheme` (prefer-dark/light), `gtk-theme` (Adwaita/Adwaita-dark) and the icon theme. There is no `accent-color`, so libadwaita apps show the default blue; it reads `'blue'` here. No gtk.css template exists | `bin/omarchy-theme-set-gnome` |
| Qt apps | `QT_QPA_PLATFORMTHEME=gtk3`, Fusion style, dark/light only. Kvantum was removed (migration `1785351479.sh`, 2026-07-29) | `default/hypr/envs.lua`, migrations |
| Web apps | Chromium `--app=` windows; the browser gets a managed `BrowserThemeColor` | `bin/omarchy-launch-webapp`, `bin/omarchy-theme-set-browser` |
| Omarchy's own Rust GTK app (gliff) | Maps `colors.toml` onto libadwaita CSS variables with a `CssProvider` and a `gio::FileMonitor` on `~/.local/state/omarchy/current/` | omacom/gliff `crates/gliff/src/theme.rs` (2026-09-19) |

**Consequence.** Every non-Quickshell toolkit must read `colors.toml` itself and
re-skin itself. It must watch `current/`, not files inside `theme/`, because
`theme-set` swaps the whole directory. That is achievable, and gliff proves it
for GTK, but it is never "native for free".

### 3.2 Toolkit comparison

Startup and size figures are measurements on this machine where marked; no
rigorous 2025–2026 cross-toolkit benchmark was found.

| Option | Latest stable (date) | Omarchy look (1–5) | Live theme | Keyboard-first | a11y on Linux | Startup / size | Rust burden | Main risk |
|---|---|---|---|---|---|---|---|---|
| **FrankenTUI** (continued) | ftui **0.9.0** (2026-10-03); fbittorrent pins 0.6 (`Cargo.toml:24-25`); 0.6.0 2026-08-24, 0.7.0 2026-09-08 | 2: terminal cells in themed Ghostty; not panel chrome | Yes (already live) | 5 | ftui-a11y is "Phase 1" (tree types only); terminal screen readers | Instant; part of the existing 21.6 MB binary | Low (Rust only), but `src/tui/mod.rs` is 3,504 lines | API churn (five minors in four months) |
| **Quickshell QML plugin** | Quickshell 0.3.2 (2026-10-08), Qt 6.12 | **5**: same components and tokens | Yes (pushed by theme-set) | 4–5 (`PanelKeyCatcher` model) | Qt Quick AT-SPI (untested) | Loads into the running shell (no process start); QML only | QML/JS, not Rust | In-process with the desktop shell; pre-1.0 |
| gtk4-rs + libadwaita (+ Relm4) | gtk4 0.11.5 (2026-09-20), libadwaita-rs 0.9.2 (2026-07-07), Relm4 0.11.0 (2026-04); GTK 4.24.1 / adw 1.10 on Arch | 3: palette via CSS variables (gliff); Adwaita shapes and headerbar | Yes, app-side | 4 | Good (AT-SPI) | gliff 5.0 MB measured; links system GTK | Medium (GObject boilerplate) | Adwaita look; a third UI technology next to QML and the TUI |
| Qt QML via cxx-qt | cxx-qt 0.10.0 (2026-08-24); qmetaobject-rs stagnant (0.2.10, 2023) | 4: same engine, but `qs.*` and `Quickshell` imports do not exist outside the shell | Yes, app-side | 4 | Qt AT-SPI | Qt already installed | **High** (C++ toolchain, bridge, two languages) | Effort; components re-implemented |
| Slint | 1.18.1 (2026-09-21) | 3: Fluent default; palette roles read-only, so custom widgets are needed | Partial | 3–4 | AccessKit | Not measured | Low–medium | GPLv3 or attribution licence; no Wayland file drop on winit |
| iced (libcosmic) | 0.14.0 (2025-12-07) | 3 | Yes (custom Theme) | 3 | **None** (#552 open) | Not measured | Medium–high | No a11y; table not virtualized |
| egui / eframe | 0.37.0 (2026-10-15) | 2: distinctive egui look | Yes | 3 | Basic (AccessKit) | Not measured | Low, monthly breaking releases | Hyprland data-device conflict (winit #4729); look |
| Tauri 2 | 2.12.4 (2026-10-14) | 4 (CSS) | Yes | 5 (DOM) | Good | Webkit processes | Medium (JS + IPC) | GTK3 + WebKitGTK 4.1; NVIDIA/DMABUF issues |
| Localhost web UI in an Omarchy webapp window | Chromium 154 via `omarchy-launch-webapp` | 3–4 (CSS can match colors; browser window and web feel) | Yes (SSE + CSS variables) | 5 (DOM) | Good | Hands off to the running Chromium over its singleton socket | Medium (HTTP server on Asupersync + JS frontend) | Localhost HTTP security (CSRF, DNS rebinding), the same lesson as the qBittorrent WebUI defaults |
| GPUI (+ gpui-omarchy) | official `gpui` 0.2.2 is stale; `gpui-pre` 0.3.9 (2026-10-12, third-party snapshot); gpui-omarchy 0.2.1 | 4–5: built for Omarchy (tobi/disktree) | Yes | 5 | AccessKit (new) | disktree 29 MB | High (weekly churn) | Supply chain: one third-party publisher |

**Cross-cutting.** winit 0.30 (stable) reports **no file drops on Wayland**;
support exists only in 0.31 betas (winit PR #4571). That affects egui, iced and
Slint's winit backend. Dioxus desktop inherits the WebKitGTK caveats.

### 3.3 FrankenTUI continued

What a richer TUI could still gain:

- **What ftui 0.7 has** (`ftui-widgets-0.7.0/src/`, `ftui-extras-0.7.0/src/`
  in the local cargo registry):
  - `Table`, virtualized lists, `Tabs`, `Tree`;
  - `CommandPalette`, `Toast`/`NotificationQueue`, modal, `FilePicker`;
  - drag and keyboard-drag, `TextArea`, `LogViewer`;
  - extras charts (`Sparkline`, `BarChart`, `LineChart`), canvas, markdown;
  - images with Kitty, iTerm2, Sixel and ASCII protocols;
  - OSC 52 clipboard.
- **What fbittorrent already uses:** mouse hover, click, double-click and wheel
  (`README.md:24-29`), plus kitty-graphics posters (`src/tui/poster.rs:1-60`).
- **The ceiling:**
  - no real windows or shadows;
  - typography and spacing fixed to the terminal grid;
  - no drag-and-drop of `.torrent` files from a file manager;
  - posters only in kitty-protocol terminals;
  - it can never use Omarchy's panel chrome.

It can look *good*, but it cannot look like the Wi-Fi panel.

### 3.4 Takeaway for task B

- **Every Rust toolkit is a third UI technology.** Omarchy's bar integration
  must be QML anyway, so a Rust GUI would sit next to the QML plugin and the
  TUI, and none of them can reuse Omarchy's shell components.
- **The best non-QML candidate is gtk4-rs + libadwaita with gliff-style
  theming.** It has the best Wayland, DnD, IME and AT-SPI support. It still
  looks like a GNOME app re-tinted in Omarchy colors, not like the shell.
- **The cheapest is a localhost web UI in an Omarchy webapp window.** It
  carries an HTTP security surface the project already learned to fear.
- **Neither beats a QML dashboard on the owner's stated goal:** the look and
  feel of OmaTorrent and the Omarchy panels.

---

## 4. Topology options (task C)

### 4.1 The three topologies

**T1 — engine stays inside the TUI; a Quickshell UI drives it over IPC.** This
is the current pattern, extended.

**T2 — headless engine daemon.** A systemd user service started on demand.
The TUI, the Quickshell UI and the CLI are all clients.

**T3 — the engine is a library linked into a GUI binary.** The pattern of
Transmission GTK, the qBittorrent GUI and Deluge standalone. The TUI, CLI and
widget are clients of the GUI process.

| Criterion | T1 engine in TUI | **T2 daemon** | T3 engine in GUI binary |
|---|---|---|---|
| "Nothing at login" | Yes | Yes. The unit is never enabled; clients run `systemctl --user start` | Yes |
| What opening a UI costs | Every UI needs a terminal window to exist first | Nothing. The dashboard connects, starting the daemon if needed | The GUI window must exist; the TUI and widget are useless without it |
| `magnet:` click | Opens a terminal (`src/backend.rs:23,62-86`) | Adds silently, notifies, and can show a popup confirm | Opens the GUI window |
| Downloads after closing UIs | Stop | **Policy choice** (§7.4) | Stop, unless hide-to-tray. Omarchy has no tray idiom; the owner rejected a tray icon |
| Crash isolation | A TUI panic or a closed terminal kills transfers | Engine isolated from the shell, TUI and CLI. A daemon crash restarts via systemd; UIs show "disconnected" | A UI crash kills transfers |
| VPN binding | Works. The TUI tick drives the transition check (`src/tui/mod.rs:1701`) | Works the same (per-socket `bind_device_name`). The watch moves into the daemon loop; parked state is broadcast to every UI | Works; same as T1 |
| Multi-frontend consistency | Single engine owner, but RSS, search and shows run per client process | **Best**: one owner of every feature, one event stream | Single owner, but every other UI depends on one window |
| IPC versioning | Needed (old TUI vs new CLI) | Needed and more visible: a long-lived daemon meets an upgraded binary (§7.2) | Needed |
| Effort from today | IPC events + dashboard; no lifetime work | Daemon S, API v1 M, then any UI | New GUI binary, plus moving engine ownership to it |
| Prior art | Fragments (daemon is a child of the window), Transmission GTK | transmission-daemon, qbittorrent-nox, deluged thin client, rqbit server, torrentTUI `--headless` | Transmission GTK/Qt local, qBittorrent GUI, Deluge standalone, rqbit desktop |

**Verdict: T2.**

- **T1 contradicts the reason for a graphical UI.** With T1, a dashboard user
  still gets a terminal window.
- **T3 recreates today's coupling with a heavier process,** and every other
  frontend depends on one window. Deluge shows the failure mode: standalone mode
  meets a running `deluged` and errors with `DaemonRunningError`
  (`deluge/ui/gtk3/gtkui.py:300-350`).

### 4.2 Daemon lifetime mechanics (T2)

- **Start.** Any client that needs the engine runs
  `systemctl --user start fbittorrent.service`: `add`, `up`, the TUI, the
  dashboard's "Start" control, and the menu. `status` and the bar's summary
  connection do not start it. `systemctl` serializes concurrent starts, which
  removes today's 60 s poll loop.
- **No socket activation.** rqbit ships a socket unit, but here it would make
  the bar's status connection start the engine. That breaks the "status never
  starts it" contract (`src/main.rs:61`).
- **Single owner.** Keep the flock (`src/ipc.rs:130-163`). librqbit's
  persistence cannot tolerate two owners (prior art, §5).
- **Stop.** An explicit Quit (TUI `Q`, widget power, `fbittorrent down`, menu)
  sends `app.shutdown`. SIGTERM from systemd follows the same drain path.
- **Idle exit.** Optional, per the policy in §7.4.
- **Upgrade.** `hello` returns the daemon's binary version. A client with a
  different version tells the user, or restarts the daemon when idle.
- **Sleep.** Hold a logind sleep inhibitor while downloading. This is the
  qBittorrent path that works on Hyprland; Transmission GTK's GNOME-only path
  does nothing there.

---

## 5. Prior art (task D)

| Project (latest stable, date) | Topology | Update model | Versioning | Notes |
|---|---|---|---|---|
| Transmission 4.1.3 (2026-06-30) | Daemon + GTK (in-process engine) + Qt (local or remote) + web + `transmission-remote` | Polling. `torrent_get` with `fields` and `ids:"recently_active"` (60 s window plus a `removed` list). Qt polls every 3 s with a full refresh every 60 s; the web UI every 5 s | JSON-RPC 2.0, snake_case since 4.1 behind a compat layer; `rpc_version_semver`, also sent in the 409 CSRF reply | Unix-socket RPC (`unix:` bind, mode 0750). Qt sends the same RPC in-process via `tr_rpc_request_exec` |
| Fragments 3.0.1 (2024-05; main active 2026-09) | GTK4/libadwaita Rust app spawning `transmission-daemon` as a child; killed on close (pdeathsig on Flathub) | Adaptive polling: 1.5 s local, 5 s when the window is inactive, 30 s when suspended; files and peers only for an open details view | Transmission RPC | Clipboard magnet toast with dedupe; DnD overlay; suspend inhibit; stops on metered networks or VPN disconnect, with the reason as an enum |
| rqbit v9.0.1 (2026-08-20) | Library + `rqbit server` (axum HTTP + React UI) + Tauri desktop (in-process) | Polling: web UI every 1 s while active, 5 s idle; no deltas. Only `/stream_logs` streams | Root `GET /` returns its version; per-endpoint `/stats/v1` | Socket-activated user unit. The library has `state_change_notify` and `have_broadcast_tx` hooks a push stream could use. One shared TS interface over HTTP or Tauri invoke |
| Deluge 2.2.0 (2025-04-28) | `deluged` + GTK (standalone = in-process) + web + console | **Push events** (22 types, per-client interest) plus polled stats (1.5 s cache) | DelugeRPC (rencode over TLS) | Standalone-vs-daemon collisions; "Quit & Shutdown Daemon" |
| qBittorrent 5.2.4 (2026-09-28) | GUI (in-process) or `qbittorrent-nox` + WebUI | **`sync/maindata?rid=` acknowledged deltas** with a per-client snapshot, `full_update` and `*_removed` lists; WebUI polls every 1.5 s | `/api/v2` + `webapiVersion` 2.15.1 + `WebAPI_Changelog.md` | No push (WebSocket request #22086 still open) |
| Flood v4.16.3 (2026-10-17), qui v1.32.0 (2026-10-13) | Web layers over other clients | **SSE: snapshot, then deltas with a sequence**; resync on a gap; fast with subscribers, slow without | n/a | Closest to what a Quickshell client wants |
| torrentTUI v0.18.1 (2026-10-07) | Rust + ratatui + librqbit; TUI owns the engine by default, Ctrl+D detaches, `--headless`, a user unit | n/a | n/a | flock single owner; a second launch hands its magnet to the running session. The closest prior art to fbittorrent |
| omaqbt 2.1.3 (Omarchy marketplace) | Quickshell plugin (service + bar-widget + panel FloatingWindow) over qbittorrent-nox in a user unit | Polling the qBittorrent WebUI through a Python sidecar | qBittorrent API | Turtle mode (`z`), state filters (`a`/`p`/`c`), sort cycle, `magnet:` → popup confirm, notifications on completion |

**UI patterns to borrow** (sources in the prior-art report, `scratchpad/prior-art/`):

- State filters with live counts: Transmission's All, Active, Downloading,
  Seeding, Paused, Finished, Error, or Fragments' groups (Downloading, Queued,
  Seeding).
- Per-torrent detail tabs: Overview, Files (checkboxes, search, open), Peers,
  Trackers, a pieces bar and a speed graph (qBittorrent `proptabbar.cpp`,
  `downloadedpiecesbar.cpp`).
- An "alternative speed" (turtle) toggle in the status line.
- An add flow with file selection, destination plus free space, and start
  paused. omaqbt adds stopped first, then confirms.
- Clipboard magnet detection with dedupe against existing hashes (Fragments'
  toast).
- Notifications with "Open file" and "Open folder" actions (Transmission
  `gtk/Notify.cc`).
- A sleep inhibitor while downloading, with separate settings for downloading
  and seeding (qBittorrent).
- An explicit, one-time explanation of what closing does (torrentTUI's Ctrl+D
  confirm, qBittorrent's close-to-tray notice).
- A visible stop reason in every UI, such as "Waiting for ProtonVPN"
  (Fragments' stop-reason enum).

---

## 6. Decision matrix

**Options:**

| | Option |
|---|---|
| **O1** | TUI only, engine in TUI (status quo, polish ftui) |
| **O2** | Quickshell UI over the engine in the TUI (T1) |
| **O3** | **Daemon + Quickshell (popup + dashboard) + TUI + CLI** |
| **O4** | Daemon + Quickshell only (retire the TUI) |
| **O5** | Daemon + GTK4/libadwaita app (+ bar plugin) |
| **O6** | Daemon + standalone cxx-qt QML app (+ bar plugin) |
| **O7** | Daemon + localhost web UI in an Omarchy webapp window (+ bar plugin) |
| **O8** | GUI binary that embeds the engine (T3) |

Slint, iced, egui and GPUI are left out of the matrix. They score at or below
O5 on look, a11y or Wayland DnD (§3.2), and add the same third-technology cost.

**Criteria and weights** (5 = best):

| Criterion | Weight | What it measures |
|---|---|---|
| Look | 3 | Omarchy-native look |
| Theme | 1 | Live theme following |
| Keys | 2 | Keyboard-first operation |
| Ceiling | 2 | Feature ceiling: posters, graphs, DnD, details, notifications |
| Life | 2 | Fit with the lifetime goals: nothing at login, no terminal side effects |
| Iso | 2 | Fault isolation |
| Cons | 1 | Consistency across frontends |
| Effort | 2 | Effort from today (5 = least) |
| Maint | 2 | Long-term maintenance (5 = least) |
| Risk | 1 | Platform risk (5 = least) |
| Reach | 1 | Works over SSH, on a TTY, or without the shell |

| | Look ×3 | Theme ×1 | Keys ×2 | Ceiling ×2 | Life ×2 | Iso ×2 | Cons ×1 | Effort ×2 | Maint ×2 | Risk ×1 | Reach ×1 | **Total /95** |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| O1 | 2 | 5 | 5 | 3 | 2 | 2 | 4 | 5 | 4 | 4 | 5 | **66** |
| O2 | 5 | 5 | 4 | 4 | 1 | 2 | 3 | 3 | 3 | 3 | 5 | **65** |
| **O3** | 5 | 5 | 5 | 5 | 5 | 4 | 4 | 2 | 2 | 3 | 5 | **78** |
| O4 | 5 | 5 | 4 | 5 | 5 | 4 | 5 | 3 | 3 | 2 | 2 | **77** |
| O5 | 3 | 4 | 4 | 5 | 5 | 5 | 4 | 2 | 2 | 4 | 2 | **69** |
| O6 | 4 | 4 | 4 | 5 | 5 | 5 | 4 | 1 | 1 | 3 | 2 | **67** |
| O7 | 3 | 4 | 4 | 4 | 5 | 5 | 4 | 2 | 2 | 3 | 5 | **69** |
| O8 | 3 | 4 | 4 | 5 | 2 | 1 | 2 | 2 | 2 | 4 | 1 | **52** |

Per-cell justification:

- **O1 TUI only.**
  - Look 2: terminal grid inside a themed Ghostty, never panel chrome.
  - Theme 5: reloads `colors.toml` on mtime (`src/tui/mod.rs:779`).
  - Keys 5: built keyboard-first.
  - Ceiling 3: ftui has tables, charts and kitty posters, but no file DnD and no
    real windows.
  - Life 2: the engine dies with the terminal, and a magnet click opens a
    terminal.
  - Iso 2: a TUI panic or a closed terminal stops transfers.
  - Cons 4: one UI, but the widget polls through the CLI.
  - Effort 5: nothing to change.
  - Maint 4: Rust only, but one 3.5k-line `mod.rs`.
  - Risk 4: owned code, though ftui churns.
  - Reach 5: SSH and TTY.
- **O2 Quickshell over the engine in the TUI.**
  - Look 5 and Theme 5: shell components.
  - Keys 4: the panel model works, with window-focus quirks.
  - Ceiling 4: limited by the missing peers, pieces and events APIs.
  - Life 1: opening the GUI requires a terminal to host the engine.
  - Iso 2: as O1.
  - Cons 3: two UIs, one tied to the engine.
  - Effort 3: dashboard plus events, no lifetime work.
  - Maint 3: Rust plus QML.
  - Risk 3: Quickshell pre-1.0.
  - Reach 5: TUI kept.
- **O3 daemon + Quickshell + TUI + CLI.**
  - Look 5 and Theme 5: shell components, live theme.
  - Keys 5: the panel model plus the TUI.
  - Ceiling 5: real window, posters, graphs, DnD, details.
  - Life 5: on-demand daemon, silent magnet adds, explicit policy.
  - Iso 4: engine isolated, but the QML still shares the desktop shell's
    thread.
  - Cons 4: one owner and one event stream, but two UIs can drift.
  - Effort 2: daemon S, API M, dashboard L.
  - Maint 2: QML plus ftui UIs.
  - Risk 3: Quickshell/Omarchy alpha APIs, with the TUI as a fallback.
  - Reach 5.
- **O4 daemon + Quickshell only.**
  - Look, Theme, Ceiling, Life and Iso as O3.
  - Keys 4: loses the terminal workflow.
  - Cons 5: one UI.
  - Effort 3: no TUI upkeep, but search, RSS and shows must reach parity in QML
    before retiring the TUI.
  - Maint 3: one UI, but QML is harder to test than the tmux-driven TUI.
  - Risk 2: no UI at all if the shell API breaks.
  - Reach 2: CLI only over SSH.
- **O5 GTK4/libadwaita.**
  - Look 3: Omarchy colors via CSS variables (gliff), but Adwaita shapes and a
    separate look from the bar.
  - Theme 4: app-side watch.
  - Keys 4.
  - Ceiling 5: ColumnView, portal file chooser, DnD, AT-SPI.
  - Life 5.
  - Iso 5: its own process.
  - Cons 4.
  - Effort 2: a new UI stack, while the bar plugin still needs QML.
  - Maint 2: three UI technologies if the TUI stays.
  - Risk 4: GTK and gtk-rs are mature.
  - Reach 2.
- **O6 cxx-qt QML.**
  - Look 4: the same engine, but Omarchy's `qs.*` components cannot be
    imported.
  - Theme 4, Keys 4, Ceiling 5, Life 5, Iso 5, Cons 4.
  - Effort 1: C++ toolchain, bridge, re-implemented components.
  - Maint 1: three languages.
  - Risk 3.
  - Reach 2.
- **O7 localhost web UI.**
  - Look 3: CSS can match colors, but it is a Chromium window and feels like
    the web.
  - Theme 4: SSE plus CSS variables.
  - Keys 4.
  - Ceiling 4: DnD gives file contents, not paths.
  - Life 5, Iso 5, Cons 4.
  - Effort 2: an HTTP server on Asupersync (librqbit's axum API is compiled
    out) plus a JS app.
  - Maint 2: a JS/TS frontend plus HTTP hardening.
  - Risk 3: localhost CSRF and DNS rebinding.
  - Reach 5: an SSH tunnel or a phone.
- **O8 GUI embeds the engine.**
  - Look 3, Theme 4, Keys 4, Ceiling 5.
  - Life 2: downloads end with the window, and there is no tray idiom.
  - Iso 1: a UI crash kills transfers.
  - Cons 2: every other client depends on the window.
  - Effort 2, Maint 2, Risk 4.
  - Reach 1.

**Sensitivity.** O3 and O4 differ only in Keys, Cons, Effort, Maint, Risk and
Reach. If the owner never uses the TUI over SSH (Reach weight 0), O4 wins
75 to 73. Re-weighting does not otherwise change the leader:

- With Look at weight 0, O3 still leads (63, against 60 for O1, O5 and O7).
- Only making Effort dominant (weight 6 or more) lets the status quo O1 tie
  or pass O3.

---

## 7. Recommended topology and API shape

### 7.1 Diagram

```
 user session (Hyprland + omarchy-shell)
 ┌─────────────────────────────────────────────────────────────────────────────────┐
 │ omarchy-shell (single Quickshell process)                                       │
 │   oliver.torrents plugin                                                        │
 │     Service.qml  kind service   ONE Socket connection, hello + subscribe,       │
 │                                 ListModel store keyed by info hash              │
 │     Panel.qml    kind bar-widget  icon + KeyboardPanel popup (per monitor,      │
 │                                   reads the service; no polling of its own)     │
 │     Window.qml   kind panel       FloatingWindow "fbittorrent" dashboard        │
 │                                                                                 │
 │ Ghostty ─ fbittorrent tui        FrankenTUI client (same API)                   │
 │ xdg-open magnet: / menu / agents ─ fbittorrent add|up|down|… (short-lived CLI)  │
 └──────────────┬──────────────────────────────────────────────────────────────────┘
                │ NDJSON over $XDG_RUNTIME_DIR/fbittorrent/api.sock  (dir 0700, sock 0600)
                │ legacy framed protocol kept on engine.sock for one release
                ▼
 systemd --user  fbittorrent.service   (static unit, never enabled; started on demand)
 ┌─────────────────────────────────────────────────────────────────────────────────┐
 │ fbittorrent serve                                                               │
 │   instance flock (engine.lock)        API: hello, requests, subscriptions       │
 │   event bus: snapshot + deltas (rev)  engine: librqbit on Asupersync, maintain  │
 │   network watch: VPN bind / park      feeds: RSS + Shows scheduler, search jobs │
 │   IMDb cache → poster file paths      notifier (completion), logind inhibitor   │
 └─────────────────────────────────────────────────────────────────────────────────┘
   state: ~/.local/share/fbittorrent/{engine/,rss.json,shows.json,…}  ~/.cache/fbittorrent/imdb
```

### 7.2 API v1 shape

**Transport and framing**

- One JSON object per line, UTF-8, `\n`-terminated, with a maximum line length
  of about 1 MiB. This is what `Quickshell.Io.Socket` + `SplitParser`, the
  TUI's thread, and the CLI can all speak.
- `.torrent` uploads go by path (`{"path": "/…/x.torrent"}`; same user, same
  machine), or as base64 for drag-and-drop content.
- The CLI and the daemon ship in one binary, so the old framed protocol can be
  removed once the new socket ships. Keeping `engine.sock` for one release
  covers an old daemon still running during an upgrade.

**Handshake and versioning**

```
→ {"id":1,"op":"hello","proto":1,"client":"omarchy-plugin/0.2"}
← {"id":1,"ok":true,"result":{"proto":1,"proto_minor":0,"server":"fbittorrent 0.2.0",
     "engine":"librqbit 9.0.1","instance":"<uuid>","state":"running|parked|stopping",
     "caps":["torrents","files","peers","pieces","rss","shows","search","imdb","settings"]}}
```

- **Major `proto`:** the server refuses an unknown major with a typed error, so
  the client can say "update the plugin" or "restart fbittorrent".
- **`proto_minor` and `caps`:** additive changes only. Clients ignore unknown
  fields. Methods are namespaced, and deprecations are announced in a changelog
  (qBittorrent's `WebAPI_Changelog.md` model).
- **`server`** carries the binary version, so a CLI can detect "daemon older
  than me" after an upgrade.
- **Errors:** `{"id":n,"ok":false,"error":{"code":"not_found","message":"…",
  "hint":"…"}}`, reusing the CLI's existing code names
  (`src/agent.rs` capabilities, exit-code dictionary).

**Requests**

These map onto today's routes plus the missing pieces.

| Namespace | Ops |
|---|---|
| `app` | `hello`, `ping`, `version`, `shutdown`, `restart`, `settings.get`, `settings.set` (`max_active_downloads`, `peer_encryption`, `network_interface`, `seed_by_default`, `save_path`), `network` |
| `torrents` | `list`, `get`, `add {source: magnet\|url\|path\|base64, stopped, seed, save_path, category}`, `start`, `stop`, `delete {delete_files}`, `files`, `setFiles {include/skip}`, `setSeed {on/off}` (wraps today's upload-limit, share-limit and tag trio, `src/qbt.rs:504-520`), `setUploadLimit`, `tags`, `peers`, `pieces` (run-length-encoded haves), `trackers`, `openTarget` (returns a path; the client opens it) |
| `transfer` | `info`, `setAltSpeed {on}` (turtle) |
| `search` | `providers`, `setProvider`, `start {query}` (results stream on the `search:<id>` topic), `stop`, `browse {provider, list}`, `resolve` |
| `rss` | `feeds`, `add`, `remove`, `refresh`, `items`, `setRead`, `grab` |
| `shows` | `list`, `follow`, `unfollow`, `check`, `episodes`, `grab`, `setSeen` |
| `imdb` | `lookup {name\|id}` → `{rating, blurb, url, poster_path}`. The daemon fetches and caches; QML loads `file://` |

**Event stream**

```
→ {"id":2,"op":"subscribe","args":{"topics":["summary","torrents","network"],"interval_ms":1000}}
← {"id":2,"ok":true,"result":{"sub":1}}
← {"ev":"snapshot","sub":1,"rev":1042,"instance":"…","data":{"summary":{…},"torrents":[…],"network":{…}}}
← {"ev":"delta","sub":1,"rev":1043,"changed":{"torrents":{"<hash>":{"progress":0.43,"dlspeed":4123456}}},
     "removed":{"torrents":[]},"summary":{"dl":4123456,"up":32768,"active":2}}
← {"ev":"torrent.completed","hash":"…","name":"…","open_target":"/home/…/Downloads/…"}
← {"ev":"network.changed","state":"parked","message":"Waiting for ProtonVPN"}
← {"ev":"bye","reason":"shutdown"}
```

- **Topics:**
  - `summary`: counts and rates. This is all the bar needs; subscribe at
    5000 ms while the popup is closed.
  - `torrents`: list fields only.
  - `torrent:<hash>`: files, peers, pieces and trackers, only while a detail
    view is open.
  - `transfer`, `network`, `queue`.
  - `rss`, `shows`: unread counts and new items.
  - `search:<id>`: streamed results.
  - `log`.
- **Delta rules:**
  - The server diffs, coalesces to `interval_ms` (clamped 250–10000), and sends
    changed fields only, plus `removed` lists.
  - Structural events (added, removed, state change, completed, error, network)
    go out immediately. librqbit's `state_change_notify` can trigger them.
  - `rev` is monotonic per `instance`. A gap or a new `instance` means the
    client resubscribes for a fresh snapshot. This is the qui and Flood model,
    with no periodic keyframes.
  - A client that stops reading is dropped after a bounded buffer.
- **Polling remains possible** (`torrents.list`) for the CLI and for agents.
  The bar never polls a process again.

### 7.3 Main risks and mitigations

| Risk | Mitigation |
|---|---|
| QML in the desktop shell process stalls the bar or lock screen | All diffing and sorting in Rust; deltas only; `ListModel` keyed by hash with `reuseItems`; the detail topic only while visible; no JS loops over every torrent per tick |
| Quickshell, Omarchy or Qt churn (0.3.x patched for Qt 6.12; `4.0.0.alpha`) | Use only documented contracts (`Panel`, `KeyboardPanel`, `PanelKeyCatcher`, `Commons.Color/Style`, the `shell` facade). Keep the TUI and CLI as fallbacks. Fallback variant: run the dashboard as a standalone `qs -p` instance with its own `//@ pragma AppId`. Omarchy's rules discourage that for *plugins* (`site/develop.html:29`), and it would have to copy theme tokens, but it buys isolation and a real app id |
| Two UIs drift | Every feature lands as an API op and topic first. The TUI and QML render it. The tmux smoke tests and a socket-level contract test cover the API |
| Lifetime semantics change (closing the UI no longer stops downloads) | Make it an explicit setting (§7.4); show the policy once on first close; a power control in the popup |
| A daemon on a different version after upgrading the binary | Version in `hello`; the CLI warns; `fbittorrent restart`; auto-restart when idle |
| Marketplace listing needs a native binary | A `manual-setup` listing; the plugin detects a missing or old binary and shows install instructions |

### 7.4 Decisions the owner must make before planning

1. **Stop policy when the last window closes.** Options:
   - (a) keep running until idle (no active downloads, no allowed seeding), then
     exit after a grace period;
   - (b) stop when the last *interactive* UI (TUI or dashboard) closes, which is
     closest to today;
   - (c) run until explicit Quit.

   The bar widget must not count as a UI. Recommended default: (a). "Nothing
   runs at login" holds under all three.
2. **Is the TUI kept long-term (O3) or retired after parity (O4)?** Recommended:
   keep it at its current scope, and decide later.
3. **Dashboard surface.** A FloatingWindow app window (recommended), an
   OverlayWindow full-screen dashboard (OmaTorrent's "Omarchy overlay" style),
   or both. The overlay suits a quick search-and-add palette; the window suits
   the long-lived torrent list.
4. **Should search, RSS and shows move into the daemon now** (recommended, so
   QML never shells out per action), or should the QML first use CLI `--json`
   commands as a bridge?

---

## 8. What a Quickshell-native fbittorrent UI could look like

Informed by the Wi-Fi, Bluetooth and Tailscale panels (`PanelHero`,
`PanelSectionHeader`, `PanelActionButton`, `ToggleSwitch`, `ConfirmDialog`,
`FilterInput`), the dev-gallery window, omaqbt, and the TUI review's
one-interaction-model table (`docs/tui-ux-review.md`).

### 8.1 Plugin structure

```
plugin/oliver.torrents/
  manifest.json   kinds ["service","bar-widget","panel"]; entryPoints service/barWidget/panel
  Service.qml     the one API connection (Socket + SplitParser + reconnect Timer), ListModels,
                  action queue, notifications of errors; ShellIpc target "oliver.torrents"
  Panel.qml       BarIconButton + KeyboardPanel popup; reads `service`
  Window.qml      FloatingWindow dashboard; open(payload) picks the view: {"view":"search","query":"…"}
  views/          TorrentList, Details (Overview/Files/Peers/Trackers/Pieces), Search, Shows, Rss, Settings
  Model.js        pure formatting helpers (human sizes, ETA, state → glyph/colour)
```

- **Summon:** `omarchy-shell shell summon oliver.torrents '{"view":"torrents"}'`.
  This opens the window, because panel kind wins (§2.2). The popup stays on
  `omarchy-shell oliver.torrents toggle`.
- **Hyprland rule** by title, for example
  `o.window({ class="^org.quickshell$", title="^fbittorrent$" }, { float=true, size=… })`.
- **Menu:** `torrents.open` points at the summon. A new free SUPER+CTRL chord
  (T is taken) toggles the window.

### 8.2 Bar icon and popup (glance and act; about 400 px, as today)

```
┌─────────────────────────────────────────────┐
│ 󰇚  Torrents                            [⏻] │  PanelHero: meta "2 downloading · ↓ 4.2 MB/s"
│    2 downloading · ↓ 4.2 MB/s  ● proton0    │  detail pill = VPN state; trailing power toggle
│ ─────────────────────────────────────────── │
│ DOWNLOADING                                 │  PanelSectionHeader
│ The Diplomat S04 1080p …   ▕████▌    ▏ 43%  │  row: name, thin progress bar, ETA on hover
│ Some.Movie.2026 …          metadata…        │  hover/cursor shows ⏸  📂  ✕ PanelActionButtons
│ QUEUED 1 · DONE 8 · PAUSED 1          ›     │  collapsed groups; Enter expands
│ ─────────────────────────────────────────── │
│ 󰐕 Add from clipboard                    a   │  wl-paste → torrents.add; Fragments-style dedupe
│ 󰕮 Open fbittorrent                      w   │  summons Window.qml
│ 󰍉 Search…                               /   │  summons Window.qml {"view":"search"}
└─────────────────────────────────────────────┘
```

- **Bar icon states:**
  - idle;
  - downloading, optionally with compact ↓/↑ next to the glyph as in omaqbt;
  - waiting for the VPN, in the urgent colour;
  - error.
- **Clicks:** left toggles the popup, right adds from the clipboard, middle
  opens the window.
- **Visibility:** the widget stays visible only while the daemon runs, as
  today. Alternatively it could stay always visible with the power toggle; the
  owner decides.

### 8.3 Dashboard window (the long-lived surface)

```
┌ fbittorrent ─────────────────────────────────────────────────────────────────────────────┐
│ 1 Torrents   2 Search   3 Shows ●4   4 RSS          [/ filter or search…]   ↓4.2M ↑32K ●VPN │
├───────────────┬──────────────────────────────────────────────────────┬──────────────────┤
│ All        12 │ NAME                           SIZE  PROGRESS   ↓     ETA │ ┌──────┐        │
│ Downloading 2 │ ▸ The Diplomat S04 1080p …    3.4G ▕███▌ ▏43%  4.1M  12m │ │poster│ The    │
│ Queued      1 │   Some.Movie.2026.1080p …     2.1G  metadata…              │ └──────┘ Diplomat│
│ Seeding     0 │   Linux.ISO.x86_64            1.2G ▕█████▏done             │ ★ 7.9 · 2023–    │
│ Done        8 │                                                          │ blurb …          │
│ Paused      1 │                                                          │ i IMDb  w Wiki   │
│ Errors      0 │                                                          ├──────────────────┤
│ ───────────── │                                                          │ Files Peers Trk ▤│
│ Shows  ●4     │                                                          │ ☑ S04E01.mkv 1.1G│
│ RSS    ●2     │                                                          │ ☐ sample.mkv     │
│               │                                                          │ ▕▇▇▇▁▁▇▇▇▁▁▁▁▏   │
├───────────────┴──────────────────────────────────────────────────────┴──────────────────┤
│ a add  p pause  x remove  Enter files  i IMDb  u seed  z turtle  ? keys      ↓ ▁▂▅▇▆▃▂  │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

**Views** (the same four screens as the TUI, `1`–`4`):

- **Torrents:**
  - a sidebar of state filters with counts (Transmission, qBittorrent),
    Shows/RSS unread badges and categories or tags;
  - a virtualized list with progress bars;
  - a detail pane: Overview with the IMDb card and poster from the daemon's
    cache, Files with include/skip checkboxes, Peers, Trackers, a pieces bar
    (`torrent:<hash>` topic) and a speed sparkline in the status line.
- **Search:**
  - results stream in as they arrive, deduped by info hash, with IMDb rating
    and uploaded columns;
  - an empty query opens the provider browser (TPB top-100, EZTV, YTS) as a
    grid of posters;
  - Enter downloads.
- **Shows:** the All-unread inbox first; per-show history; `m` toggles read,
  `Shift+M` marks a show read, `u` undoes. These follow the TUI semantics in
  `docs/tui-ux-review.md`.
- **RSS:** feeds and items, with the same read model.
- **Settings sheet** (F6 equivalent): max active downloads, encryption and the
  VPN interface, chosen from the interfaces returned by `app.network`.

**Add flow:**

- Sources: a magnet or `.torrent` dropped on the window (`DropArea`), the
  clipboard, the `magnet:` handler, or the CLI.
- The torrent is added stopped while its metadata resolves. A sheet then shows
  name, size, files with checkboxes, destination with free space, and seed
  on/off (the F4 equivalent). Enter starts it; Esc cancels and removes it (the
  omaqbt pattern).

**Notifications** come from the daemon (completed, error, new episodes), with
"Open file" and "Open folder" actions.

### 8.4 Keyboard model

- **Panel conventions** (`PanelKeyCatcher`):
  - `j`/`k`/arrows move, `h`/`l` move between panes;
  - Tab cycles sidebar → list → details;
  - Enter is the primary action for the selected object, using the TUI review's
    table (files for a torrent, download for a release, open items for a feed);
  - Space pauses or resumes, or toggles a file checkbox in Files;
  - `x` removes, with a `ConfirmDialog` when files would be deleted;
  - Esc clears the filter, then closes the details, then closes the window.
- **Same letters as the TUI:**
  - `a` add, `p` pause, `d`/`x` delete, `u` seed toggle;
  - `i` IMDb, `w` Wikipedia, `o` open, `r` refresh, `f` follow show;
  - `m`/`Shift+M` read state, `/` search or filter, `1`–`4` views, `?` help;
  - `z` turtle mode, borrowed from omaqbt.
- **Quitting:**
  - `Ctrl+Q` quits the engine and is labelled as such;
  - closing the window obeys the stop policy (§7.4).
- **Text fields:** while the filter or search field has focus, the catcher is
  `blocked`. `FilterInput` keeps IME composition working.
- **Focus:** a programmatic open must request activation (omaqbt `WmFocus.qml`
  pattern), so that summoning via a keybinding lands keyboard focus in the
  window.

---

## Sources

**fbittorrent** (working tree, 2026-10-18)

- `src/ipc.rs`, `src/qbt.rs`, `src/network_app.rs`, `src/backend.rs`,
  `src/main.rs`, `src/engine.rs`, `src/tui/mod.rs`, `src/search.rs`,
  `src/rss.rs`, `src/shows.rs`, `src/config.rs`
- `plugin/oliver.torrents/*`, `README.md`, `Cargo.toml`
- `vendor/rqbit/librqbit/src/{lib.rs,http_api/}`

**Omarchy**

- Installed `/usr/share/omarchy/{shell,bin,default}`.
- github.com/basecamp/omarchy @ `4e059acff2f4` (branch `quattro`):
  `docs/omarchy-shell.md`, `agents/skills/shell-dev.md`,
  `manual/32-shell-plugins.md`.
- github.com/omacom/omarchy-plugin-marketplace @ `32a11355bd7e`:
  `SUBMISSION.md`, `SECURITY.md`, `AGENTS.md`,
  `scripts/build-catalog.mjs`, `site/develop.html`.
- github.com/omacom/gliff `crates/gliff/src/theme.rs`.

**Quickshell**

- git.outfoxxed.me/quickshell/quickshell tag v0.3.2 (`4f508be`, 2026-10-08):
  `src/window/floatingwindow.*`, `src/io/{process,socket,datastream}.*`,
  `src/launch/launch.cpp`, `src/core/qmlglobal.hpp`, `BUILD.md`, changelogs.
- Installed qmltypes under `/usr/lib/qt6/qml/Quickshell/`.
- quickshell.org/docs/v0.3.0.

**Plugin examples**

- `~/.config/omarchy/plugins/io.github.grichard99.omaproton-vpn` (1.4.2).
- github.com/Aweiward/omaqbt @ `3b31e135` (2.1.3).
- AvengeMedia/DankMaterialShell @ `47885e49`.
- caelestia-dots/shell @ `71c28332`.

**Toolkits** (crates.io, GitHub releases, 2026-10-18)

- gtk4 0.11.5, libadwaita 0.9.2, relm4 0.11.0.
- cxx-qt 0.10.0, slint 1.18.1, iced 0.14.0, egui 0.37.0, tauri 2.12.4.
- dioxus 0.7.10, gpui-pre 0.3.9, gpui-omarchy 0.2.1.
- ftui 0.9.0; winit PR #4571 and issue #4729.
- Arch: gtk4 4.24.1, libadwaita 1.10.0, qt6-base 6.12.0, webkit2gtk-4.1 2.54.1.

**Prior art**

- transmission/transmission 4.1.3: `docs/rpc-spec.md`, `qt/Session.cc`,
  `gtk/*`, `daemon/*`.
- gitlab.gnome.org/World/Fragments @ `270d9be`: `src/backend/*`,
  `src/ui/window.rs`.
- ikatson/rqbit v9.0.1: `crates/librqbit/src/http_api/handlers/mod.rs`,
  `desktop/`, `systemd/`.
- deluge-torrent/deluge 2.2.0: `rpcserver.py`, `ui/client.py`, `gtk3/*`.
- qbittorrent/qBittorrent 5.2.4: `src/webui/api/synccontroller.cpp`,
  `WebAPI_Changelog.md`.
- jesec/flood v4.16.3: `server/middleware/clientActivityStream.ts`.
- autobrr/qui v1.32.0: `internal/api/sse/delta.go`.
- thijsvos/torrentTUI v0.18.1.
