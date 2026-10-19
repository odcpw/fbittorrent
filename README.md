# fbittorrent

A small torrent app for Omarchy, with an embedded Rust engine, a native bar
widget, a FrankenTUI app, and a CLI that doubles as the `magnet:` handler.
The native engine can run headlessly as `fbittorrent serve`. Write commands
start its static systemd user unit on demand; closing a client window leaves
the daemon running. `down` stops it. Torrent records and feed history stay on disk.

- **Streaming priority is automatic.** The native engine downloads each file
  sequentially and prioritizes its first and last pieces. These policies cannot
  be toggled off in native mode.
- **Ready to watch.** The largest selected video gets a playback badge after its
  verified contiguous prefix reaches 32 MiB or 3% of the file, whichever is larger,
  and its last piece is present. The row says when playback may stall. Known MKV/MP4
  duration supplies the buffer estimate; unknown duration stays unknown. The
  notification's Open action plays the file. Agents can use
  `fbittorrent wait <hash> --until playable --json` or `--until stall-free`.
- **Seeding off by default.** New torrents get a 32 KiB/s upload cap while
  downloading and stop when complete. This limits uploaded payload; it does not
  guarantee zero upload. Allow seeding per torrent with `u` in the TUI or widget,
  or `fbittorrent seed <hash>`; that removes the cap and allows completed torrents
  to keep running.
  Flip the default with `seed_by_default = true` in the config.
- **App lifetime.** CLI writes start the daemon on demand without opening a
  terminal. `up` starts only the engine; `up --ui` and `dashboard` also open the
  dashboard or TUI. Reads and status queries never start it. Nothing runs at
  login: the user unit is static. `down` stops the owner and drains the engine.
  The TUI starts or attaches to that daemon in the current terminal, including
  over SSH. `q` or Ctrl+C closes only the TUI; transfers continue. `Q` stops
  fbittorrent, asking for lowercase `y` when transfers are active. `down` also
  stops the daemon. The first `q` prints a one-time background-transfer notice
  after restoring the terminal; its preference lives in
  `$XDG_STATE_HOME/fbittorrent/notices.json`.
- **Bar widget** with the same chrome as the Wi-Fi panel: rates, progress rows,
  pause/remove, add from clipboard, one click to the TUI. It only shows while the
  app is open. Hover clipped names or details to see their full text.
- **TUI** with four screens: (1) Torrents, (2) Search (streamed provider results),
  (3) Shows (your local show list and releases), and (4) RSS (feeds and items).
  IMDb card per title:
  rating, blurb, poster; `i` opens IMDb, `w` opens the Wikipedia article. Hover
  a row to preview its full name; click selects it, double-click activates it,
  and the wheel moves the list under the pointer. Hover leaves the keyboard
  target unchanged. Selected names stay still. Hovered, unselected names scroll once after a
  short pause, return to the start and stop. `reduce_motion = true` in the
  config, or Reduce motion in Settings, disables hover scrolling.
  The bottom rows show contextual commands such as `(o) Open`; uppercase keys
  use Shift, and separate action feedback.
  Press `?` for scrollable help; `1`–`4` or Tab switch screens.
- **Local show watchlist.** Follow a title or IMDb id with `n` on Shows, `f` on
  a Search/RSS item, or `F` on a torrent. Public EZTV supplies episodes and
  season packs, with no account or external watchlist. Checks run on app opening
  when the last check is over 30 minutes old, or when you press `r`/`R`.
  Enter downloads the selected release; checking never downloads automatically.
  Read markers, download history, and cached releases persist across runs, so
  new releases can appear after years away. Shows opens in **All unread**, a
  combined inbox across every followed show, newest releases first. The Shows
  tab displays the unread count from every screen. Left selects one show to
  browse its full release history. A season pack has a key such as `S02`; an
  episode has `S02E03`. Counts mean unread, including old releases from a first
  check. In the inbox, `m` marks one release read and removes it; `u` undoes the
  last inbox read action. In one show's history, `m` toggles the selected
  release's read state and `Shift+M` marks that show's releases read.
  Select a followed show first for `Shift+M`; it is unavailable in All unread.
  The unread dot remains independent of download status. Checking, unchecked
  shows and source warnings remain visible, including when the inbox is empty.
- **Local RSS history.** Opening the app loads the cache, then refreshes each
  feed once. `r` refreshes again; Enter downloads an item and marks it read after
  a successful add. Hovering or selecting leaves read state alone. Read ids
  remain remembered when an item leaves the visible cache.
  RSS uses the same read shortcuts: `m` toggles one selected item, `Shift+M`
  marks the selected feed's items read, and Enter on a feed opens its items.
- **Provider browser.** Enter on an empty search box (or `b`) lists: 1337x
  (trending, top movies, top TV, opened in the browser), YTS latest, EZTV latest
  episodes, and The Pirate Bay top-100 movies and TV. Lists that load in the TUI
  carry IMDb ids, so the poster card appears as soon as the cursor rests on a
  title. Hover lookup also works on the Torrents screen and on RSS items.
- **Pick your providers.** `s` on the Search screen opens a checkbox list of the
  installed search plugins; the choice is stored locally and applies to
  every search. Results show IMDb ratings where space allows: instant for
  sources that carry IMDb ids (browse lists), filled in for others once their
  title lookup completes.
- **Add from anywhere.** Magnet links and `.torrent` files from the browser (the
  desktop handler owns both), the clipboard, the TUI (`a`, with F4 to change
  seeding for that add), or the CLI.
- **Dashboard add review.** Press `a` or Ctrl+V to review a clipboard magnet,
  torrent URL or local `.torrent` file. Download (Enter) confirms; Discard (Esc)
  keeps files. Without the daemon staging capability, the sheet says that metadata
  arrives after Download and creates no record before confirmation. With staging,
  it resolves metadata first and offers file selection, seeding and destination.
  The desktop handler respects `handler = "confirm"` by opening this sheet;
  `handler = "add"` adds immediately. A local file drop uses the same sheet;
  file-manager drag and drop on Hyprland still needs manual qualification.
- **File selection.** Enter on a torrent opens its file list: Space toggles a
  file between included and skipped, `1` includes it, and `a`/`n` select all or
  none. Native mode does not support high/max file priorities.
- **Download queue.** Three downloads run at once by default. Extra downloads
  wait their turn; completed seeders do not take a download slot. A manual pause
  stays paused after reopening. Press **F6** to change the limit.
- **Peer encryption.** Encrypted connections are preferred by default. F6 also
  offers Require (reject unencrypted peers) and Off. Encryption makes peer
  traffic less easily readable; other peers can still see your IP address.
- **ProtonVPN binding.** Choose ProtonVPN in F6 to require its `proton0` tunnel
  for peers, discovery, trackers and torrent-file downloads, including their DNS
  lookups. If the tunnel disappears, transfers stop and the app shows
  **Waiting for ProtonVPN**. Reconnecting reopens the engine and restores saved
  download intent. Feed, search and title-information websites use the usual
  connection. Provider download helpers that fetch a torrent outside the engine
  are refused with binding enabled; use a magnet or direct `.torrent` URL.
- **Themed** from the active Omarchy theme, live, no template needed.

## Install

```bash
scripts/install.sh                       # locked release build, atomic replacement, setup --magnet --plugin=link
fbittorrent setup --magnet --plugin=copy  # stable plugin copy instead of a checkout symlink
fbittorrent setup --magnet --plugin=link --dry-run --json  # exact plan, no changes
fbittorrent search --install              # provider scripts; Python 3 required
```

Setup writes the static user unit, desktop/MIME entries and icon, places the
plugin in the bar only when missing, and updates only `torrents.*` menu entries.
Open and Search summon the dashboard; Terminal keeps the TUI. Stop is shown
while `fbittorrent status --quiet` succeeds. Repeating setup with the same
options leaves files and commands unchanged. Nothing starts at login.

For a private prefix or a build already produced by CI, use
`scripts/install.sh --prefix /path/to/prefix --binary /path/to/fbittorrent` with
an isolated HOME and XDG directories. The default prefix is `~/.local`.

`fbittorrent setup --uninstall` removes desktop integration and keeps saved
records, RSS, Shows and downloaded files. `setup --purge --yes` also removes
application config/cache/state/data; it keeps downloaded payloads.

Optional Hyprland Lua binding (checked free on 2026-10-19; SUPER+CTRL+T is Activity):

```lua
o.bind("SUPER + CTRL + M", "Torrents", "fbittorrent dashboard")
```

Add it to your own bindings file if desired. Setup does not change keybindings
or reload your shell.

## Lifecycle

`fbittorrent serve` owns the native engine in a headless daemon. `up` starts it,
`down` drains it, and `restart` replaces the daemon and waits for a new hello.
The API can also replace just the engine slot while its sockets stay
available. `dashboard` summons the desktop view. The static user unit starts on
writes; nothing is enabled at login. Reads (`status`, `list`, `show`, `triage`)
never start an owner. When it is down, saved reads report `running:false`; `stop`
and keep-files `delete` update the saved ledger under the same instance lock.
`--no-start` and `autostart=false` prevent automatic startup. Without a user
manager, run `serve` in a terminal or tmux. Automatic idle exit uses the busy
table; binary replacement requests exit 75 when idle, and systemd starts the
installed replacement.

The CLI speaks newline-delimited API v1 on `api.sock`; `docs/IPC.md` defines the
wire contract. Contract 1.2.0 keeps torrent JSON fields and adds normalized state,
checked/reason/origin and optional `eta_s`/`seeds`. Stopped `status --json` now
returns `ok:true,running:false`; `status --quiet` exits 4 when stopped. Magnets
and local metainfo added while waiting for VPN return `deferred:true` and keep
the requested intent until the interface returns. URL adds require a live engine.

## Dashboard window

The plugin exposes a separate window entry point while the bar keeps its popup:

```sh
omarchy-shell shell summon oliver.torrents '{"view":"search","query":"sintel"}'
```

The window shell routes Torrents, Search, Shows, RSS, Settings and Log. Their views
land separately; this command currently proves window routing and query focus.
The bar service owns its API connection across window closes. `q` closes the
window; Esc clears one layer and leaves the window open at the top level.

No window-manager rule is installed. The window tiles by default. To float it,
an optional [Hyprland Lua window rule](https://wiki.hypr.land/configuring/core/rules/window-rules/)
can match its initial title and Quickshell class:

```lua
hl.window_rule({
  name = "fbittorrent-dashboard",
  match = { initial_title = "^fbittorrent$", class = "^org[.]quickshell$" },
  float = true,
})
```

Search starts from a query payload or Enter in the focused query field. Results
stream from the daemon, merge duplicate hashes, and keep provider names. Press
`s` to sort Seeds, Size, Uploaded, Name or rating, and `S` to reverse; unknown
measurements stay last. `b` opens browse lists and `P` manages providers. A result
marked `+ In your list` selects its existing torrent on Enter. New results open
the shared review sheet: staged add is used when the daemon advertises it; older
daemons send `search.grab` only after Download. `y` copies a magnet, `O` opens the
source page, and `f` follows an IMDb-linked show. The table renders at most 500 rows.

The torrent Details pane has Overview, Files, and Log tabs (`[` / `]`). At 1,400
logical pixels it docks beside the list; below that, Enter opens a sheet and Esc
closes it. Drag the divider to resize it; width and sort order survive reopening
the window or restarting the shell. `h` / `l` change pane focus. In focused Files,
Space toggles the selected file, `a` / `n` include or skip the filtered files, `/`
filters names, and `o` opens the selected file. Files sort naturally by episode
number. Overview loads the daemon's local IMDb poster and shows the confirmed
seed policy. Log shares one bounded subscription and filters by torrent hash.

Run `plugin/tests/qs-dashboard.sh --details --binary /path/to/fbittorrent` for the
private-compositor acceptance cases. They use isolated state and a local IMDb
server, restart their own daemon and shell, and retain JSONL plus native captures
under `target/test-logs/qs_details/`; they never restart the owner's shell.

## CLI

Ordinary CLI commands take `--json` and print one JSON object; errors become
`{"ok":false,"code":N,"code_name":"…","error":"…","hint":"<runnable command>"}` with
the same exit code. Torrents are selected by hash prefix, a unique part of the
name, or `all`.

```
fbittorrent                          TUI on a terminal; `triage --json` when stdout is not a TTY
fbittorrent triage --json            backend, rates, torrents, warnings, next commands (one call)
fbittorrent capabilities             machine-readable contract: commands, exit codes, conventions
fbittorrent robot-docs               agent handbook
fbittorrent search <query> --json    best-seeded results with magnet links (--limit, --wait)
fbittorrent add <magnet|url|.torrent> --json   returns the hash; --wait [--timeout S] blocks until complete
fbittorrent wait <sel>... --timeout S   progress on stderr; exit 0 done, 8 timeout
fbittorrent show <sel> --json        one torrent with files and open_target
fbittorrent list | ls                all torrents (hash, %, speed, eta, S/F/U flags)
fbittorrent start|stop|seed|noseed <sel>...
fbittorrent delete <sel> [--files --yes] [--dry-run]   data kept unless --files
fbittorrent open <sel>               the file, or the folder
fbittorrent rss list|add|remove|refresh|items|grab
fbittorrent shows list|follow|unfollow|check|episodes|grab|seen
fbittorrent tui --search | --rss | --shows
fbittorrent search --install | --install-url <url> | --plugins
fbittorrent imdb <release name>      rating, blurb, poster path
fbittorrent up [--ui] | down          start daemon / drain and stop owner
fbittorrent dashboard [--view v] [--query q]   summon panel, or fall back to TUI
fbittorrent restart                  restart daemon now; wait up to 30 s for new hello
fbittorrent serve                   run the native engine headlessly in the foreground
fbittorrent status                   rates and counts; never opens the app
fbittorrent settings --max-downloads 3 --encryption prefer --vpn proton
fbittorrent settings --interfaces   current choices and available connections
fbittorrent config | setup [--magnet] | register-magnet
```

Exit codes: 0 ok · 1 failure · 2 usage · 3 safety (needs `--yes`) · 4 backend down ·
5 upstream · 6 not found · 7 ambiguous · 8 timeout.

`fbittorrent serve` keeps the native engine available to the CLI without a TUI
or a terminal window hosting it. It owns the same private `engine.sock` and
instance lock as the current app; a second owner is refused. `down`, SIGTERM,
SIGINT or SIGHUP drains the engine and removes its socket. A missing configured
interface starts it parked, with saved history and start/stop/remove-keeping-files
available; the engine reopens when that exact interface returns.

`serve --no-notify` disables desktop notifications while retaining systemd
readiness datagrams. `idle_exit_secs` defaults to 300 in the config (minimum 30,
or 0 to disable); `serve --idle-exit-secs N` overrides it. The daemon stops when
its idle countdown reaches zero. Downloads, explicit seeding, waiting downloads
behind an unavailable interface, interactive clients, search sessions and pending
mutations cancel that countdown. The bar alone never keeps it running; opening
the popup pauses the remaining time and closing it resumes that time. The
`summary` subscription reports the countdown and busy reasons. Owner shutdown
always drains, even while busy, and preserves download intent for the next start.
`setup --magnet` writes the on-demand static `fbittorrent.service` using the
current executable path, reloads the user manager and registers desktop handlers.
It never enables or starts the unit. `setup --uninstall` removes the unit while
keeping saved data. `FBITTORRENT_UNIT` selects a different unit name for isolated
harnesses. Without a user manager, commands exit 4 with the hint `fbittorrent serve`;
run it in a terminal or tmux window. Daemon exits 70 (panic) and 75 (upgrade) belong to the
unit, outside the CLI's 0–8 dictionary. API v1 is available on the private
`api.sock`: clients can subscribe to summary, torrents, files, network, settings
and events, receive consistent snapshots, then pushed changes. The CLI and attached TUI use API v1; closing the TUI leaves transfers running.

The daemon posts completion notifications with Open File / Open Folder actions.
Browser handlers use `add --origin handler`: the daemon posts Added with Undo
(keeps files) and Open window. Ordinary `add` posts a plain toast only on a TTY
or with `--notify`; `--json`, `--quiet` and agent origin suppress that toast.
Notification bodies escape markup, duplicate pending notices collapse, and four
workers drain a bounded queue. VPN notices name the selected interface and limit
rapid flaps. `notify = false` and `serve --no-notify` disable desktop delivery.
`setup` installs the monochrome download SVG under
`$XDG_DATA_HOME/icons/hicolor/scalable/apps` (normally `~/.local/share/icons/…`);
desktop entries and notifications use fbittorrent.

Daemon diagnostics are available with `fbittorrent log --json`, filtered with
`--level error|warn|info|debug` and `--source engine` (or another source).
`fbittorrent log --follow` streams NDJSON entries. These commands never start the
daemon; when it is down they point to `fbittorrent up` and the journal's longer
history (`journalctl --user -u fbittorrent`). The in-memory ring holds 2,000
entries. `app.info.clients` counts successful connections since this daemon
started; `app.stats.active_clients` reports current connections. Nothing is sent
outside the machine.

### For agents

```
fbittorrent robot-docs                                   # read once
fbittorrent triage --json                                # what is the state?
fbittorrent search "Sintel 2010" --json | jq '.results[0].fileUrl'
fbittorrent add "<magnet>" --wait --timeout 3600 --json  # {"ok":true,"complete":true,"torrents":[{open_target…}]}
fbittorrent show <hash> --json | jq -r .open_target
```

The audit that produced this surface lives in `agent_ergonomics_audit/` (scorecard,
applied changes, handoff, and `regression_tests/run.sh`).

## Config

`~/.config/fbittorrent/config.toml`

```toml
engine = "native"
peer_port = 6882
max_active_downloads = 3
peer_encryption = "prefer"           # off, prefer, require
# network_interface = "proton"       # required ProtonVPN, or an exact interface
seed_by_default = false
show_quality = "1080p"
notify = true
timeout_secs = 10
autostart = true
# save_path = "/home/you/Downloads"      # set by `setup --save`
# category = "movies"
```

`--no-start` (a global flag) or `FBITTORRENT_NO_START=1` prevents commands from
starting the daemon or legacy service. Commands that need a closed engine return
exit 4 with an opening hint. This also prevents explicit `up`, `dashboard`, `tui` and `serve` startup;
legacy `setup` refuses because its workflow requests a service start. The widget uses
`--no-start` for status polling and every torrent action. Explicit Open TUI
requests may launch the app: middle-click, `t`/`T`, the TUI header button
(including keyboard activation), and the panel's `tui()` IPC method. Native
`reconcile` is a no-op: it reports that scheduling is intrinsic without connecting.
`autostart = false` is the persistent form for ordinary torrent commands. Show/RSS lists,
read markers, provider choices, and torrent records live in
`~/.local/share/fbittorrent`; none needs a third-party account.

F6 opens the download settings in the TUI. Use up/down to select a choice,
left/right to change it, Enter to save, and Escape to cancel. You can also click
each choice. Queue changes apply immediately. Encryption and connection changes
drain and reopen the engine. `settings --vpn off` disables interface binding.
While waiting for a required VPN, saved file names and pause choices remain
available; percentages are unknown until the engine can check files again.

For explicit legacy mode, set `engine = "qbittorrent"`, install
`qbittorrent-nox`, and run `setup`. Its loopback WebUI defaults to
`url = "http://127.0.0.1:8080"`, with `backend_unit = "qbittorrent-nox.service"`.
Legacy mode supports `seq`/`first-last`, F2/F3 add flags, and normal/high/max file
priorities. Native mode always applies streaming priority and supports file
inclusion/skipping.

## Layout

```
src/main.rs        CLI (clap)
src/engine/        embedded librqbit engine, durable torrent records, legacy import
src/daemon/        headless owner, lifecycle, replaceable engine slot, signals, sd_notify
src/ipc.rs         local Unix socket connecting CLI/widget to the open app
src/qbt.rs         common client contract; native IPC or legacy WebUI
src/backend.rs     open/focus/close the app; explicit legacy service control
src/search.rs      local provider preferences and bounded Nova/Python searches
src/rss.rs         local RSS/Atom subscriptions, cache, and durable read ids
src/shows.rs       local watchlist, EZTV checks, episode/season release history
src/setup.rs       first-run setup, magnet handler registration
src/imdb.rs        IMDb suggestion endpoint, ratings dataset cache, Wikipedia blurb, posters
src/release.rs     release-name parser (title, year, episode)
src/tui/           FrankenTUI app: mod.rs (screens), theme.rs (Omarchy colors), poster.rs
plugin/oliver.torrents/   Omarchy shell bar widget (Quickshell/QML)
```

## Search providers

`fbittorrent search --install` installs the official engines (eztv, limetorrents,
piratebay, solidtorrents, torlock, torrentproject, torrentscsv) plus a curated set
from the community wiki (yts, torrentgalaxy, magnetdl, therarbg, bitsearch,
torrentdownloads, kickasstorrents, snowfl, zooqle, eztvx, nyaasi). Add any other
plugin from the [qBittorrent provider wiki](https://github.com/qbittorrent/search-plugins/wiki/Unofficial-search-plugins)
with `--install-url`. The native app owns the provider preferences and runs
Python for metadata inspection, an active search, or a selected result's
download hook when you add it. Direct magnet and torrent-file links bypass
that hook. Provider scripts
execute locally, so install scripts you trust. Private trackers usually need Jackett or Prowlarr; the
official `jackett.py` plugin works once you have one of those running.

## Provider browsing

YTS loads in the app through its JSON API; if a host is unavailable, the app
tries mirrors and offers an Open site entry. The Pirate Bay (`apibay.org`)
and EZTV also load in the TUI. 1337x stays browser-only because its Cloudflare
challenge blocks scripted requests; its entries open in your browser.

## Security

The native app uses a local Unix socket restricted to the current user; it does
not expose a WebUI. In explicit legacy mode, the WebUI listens on 127.0.0.1 only
and skips authentication for localhost, which
is what makes the CLI and widget instant. Two qBittorrent protections stay ON and
`setup` enforces them on every run: CSRF protection and host-header validation.
Without them any web page you visit could POST to `localhost:8080` and add or
delete torrents. fbittorrent sends no `Origin` header and talks to the loopback
host, so both checks pass for it while a browser-originated request gets 401.

## Tests

Run `agent_ergonomics_audit/audit/regression_tests/all.sh --help` for the local
Quickshell, visual, widget, VPN and resource opt-ins. The default gate uses
isolated app fixtures and finishes with the JSONL case summary. Run it before
each merge, including when hosted Actions is unavailable. On this workstation,
use the release lock and memory scope prescribed in `AGENTS.md`.

`.github/workflows/ci.yml` runs unit/background-worker, engine-library, native
CLI/TUI and whitespace jobs on pushes and pull requests, plus a weekly RustSec
audit. Every job retains `target/test-logs/` even on failure. A manual run can
set `force_fail` to `native/quit_restart_down` to verify the failing JSONL
artifact. The workflow's presence is not proof that a hosted run has passed.

The resource gate is local and opt-in: `FBT_TEST_RESOURCES=1`. Run it on the
reference host before each release. Its checked-in baseline contains three
idle and three verified loopback-download measurements,
with the reference machine and workload. Both use a local empty IMDb ratings
fixture; the active transfer uses the existing plaintext loopback seed.
Each CPU/RSS observation must stay at
or below the baseline median plus the larger of 10% of that median and three
times the baseline range. A different host is rejected. To deliberately record
a new reference, build `release-perf`, then run the following under the same
release lock. Review the measured values before committing them. Comparison
runs never rewrite the baseline.

```bash
python3 agent_ergonomics_audit/audit/regression_tests/resources.py target/release-perf/fbittorrent \
  --runs 3 \
  --write-baseline agent_ergonomics_audit/audit/regression_tests/baselines/resources.json \
  --output target/test-logs/resources/measurement.json
```

The daemon resource gate is separately opt-in: `FBT_TEST_DAEMON_RESOURCES=1`.
It measures 10 and 100 paused local torrents with 0, 1 and 8 summary subscribers,
three 60-second windows per configuration, plus matched CLI polling references
(about 24 minutes total). The daemon entries share `baselines/resources.json`
with the existing TUI measurements. `resources.py --daemon --matrix --runs 3
--seconds 60 --baseline <path> --output <path>` compares every CPU/RSS sample
against the same median and spread rule; `--write-baseline` records a complete
matrix deliberately. Use the reference host and the release-lock/memory scope.

`python3 scripts/dagcheck.py` and `python3 scripts/mockwidth.py` review the plan's
dependency graph and screen mockups. They run by hand, outside CI.

```
agent_ergonomics_audit/audit/regression_tests/all.sh   # default isolated gate; live suites below require opt-in
cargo test --locked -- --nocapture                    # engine records, feeds/shows/search, TUI render/interactions, legacy client
python3 agent_ergonomics_audit/audit/regression_tests/native.py  # isolated native app + real CLI/TUI actions
…/regression_tests/run.sh                              # CLI agent contract (exit codes, JSON, safety)
python3 …/regression_tests/tui_smoke.py target/debug/fbittorrent  # private daemon; 80/120/160 columns; Details, Files, prompts, screens, help, q/Q
…/regression_tests/widget_keys.sh                      # bar widget keyboard handling via wtype + the `cursor` IPC hook
```

The existing legacy CLI/TUI scripts need a startable qbittorrent-nox. The widget
test also needs the Omarchy shell running with the plugin enabled.

## Notes

- IMDb's title pages are bot-gated, so ratings come from IMDb's official daily
  ratings dataset (cached in `~/.cache/fbittorrent`, refreshed after 24h) and the
  blurb from Wikipedia's summary API. Posters are IMDb's own CDN images.
- The native engine uses the rqbit protocol code ported to Asupersync, with
  app-owned tasks, network activity, and storage work. The app's normal
  dependency graph has no Tokio runtime. Existing files are hash checked on opening. Changing
  an upload cap rebuilds that torrent under the new limit and can cause another
  check. Native MSE/peer encryption is supported; high/max file priorities,
  uTP and pure BitTorrent v2 remain unavailable.
  See [the engine evaluation](docs/engine-evaluation.md) for the implementation
  choices, performance limits, and validation evidence.
- Native RSS cache keeps up to 500 visible articles per feed; read identifiers
  persist beyond that window. Show checks merge new releases into their saved
  catalogue and preserve old releases even if the source removes them. A first
  show check has a 30-second/page budget and reports incomplete history instead
  of presenting it as complete.
- In legacy mode, search jobs in qBittorrent are bound to the WebUI session
  cookie; the client keeps one session per process for that reason.
- Personal use is legal in Switzerland. Mind your local law elsewhere.

## Keys

<!-- keys:begin -->
| Scope | Action | tui | dashboard | popup | Note |
| --- | --- | --- | --- | --- | --- |
| `global` | Turtle limits | `z` | `z` | `z` |  |
| `global` | Torrents | `1` | `1` | — |  |
| `global` | Search | `2` | `2` | — |  |
| `global` | Shows | `3` | `3` | — |  |
| `global` | RSS | `4` | `4` | — |  |
| `global` | Next screen | `Tab` | `Tab` | — |  |
| `global` | Previous screen | `Shift+Tab` | `Shift+Tab` | — |  |
| `global` | Help | `?` | `?` | — |  |
| `global` | Settings | `F6`, `,` | `,` | — |  |
| `global` | Close this UI | `q`, `Ctrl+C` | `q`, `Ctrl+C` | `q`, `Esc` |  |
| `global` | Stop fbittorrent | `Q` | `Q`, `Ctrl+Q` | `Q` |  |
| `global` | Clipboard | `c` | — | — |  |
| `global` | Refresh | `r` | `r` | — |  |
| `pane:search.browse` | Load list | `Enter` | `Enter` | — |  |
| `global` | Open dashboard | — | — | `w` | Popup: w opens the dashboard (exception 2). |
| `global` | Open TUI | — | — | `t` |  |
| `global` | Add from clipboard | — | `Ctrl+V` | — |  |
| `modal:stopped` | Start fbittorrent | `Enter` | — | — |  |
| `modal:details` | Back | `Esc`, `h` | — | — |  |
| `modal:details` | Next tab | `]` | — | — |  |
| `modal:details` | Previous tab | `[` | — | — |  |
| `modal:files` | Next tab | `]` | — | — |  |
| `modal:files` | Overview | `[` | — | — |  |
| `modal:details` | Open | `o` | — | — |  |
| `modal:details` | Pause/resume | `p` | — | — |  |
| `modal:details` | Seed | `u` | — | — |  |
| `modal:details` | Remove | `d` | — | — |  |
| `modal:details` | Delete files | `D` | — | — |  |
| `modal:details` | IMDb | `i` | — | — |  |
| `modal:details` | Wikipedia | `w` | — | — |  |
| `global` | Log | `L` | `L` | — |  |
| `modal:log` | Back | `Esc` | — | — |  |
| `modal:log` | Back | `L` | — | — |  |
| `modal:log` | Level | `l` | — | — |  |
| `modal:log` | Source | `h` | — | — |  |
| `modal:log` | Previous | `↑` | — | — |  |
| `modal:log` | Next | `↓` | — | — |  |
| `modal:log` | First | `Home` | — | — |  |
| `modal:log` | Last | `End` | — | — |  |
| `modal:log` | Files | `[` | — | — |  |
| `modal:log` | Next tab | `]` | — | — |  |
| `screen:torrents` | Previous details tab | — | `[` | — |  |
| `screen:torrents` | Next details tab | — | `]` | — |  |
| `global` | Add | `a` | `a` | `a` |  |
| `screen:torrents` | Next | `j`, `↓` | `j`, `↓` | `j`, `↓` |  |
| `screen:torrents` | Previous | `k`, `↑` | `k`, `↑` | `k`, `↑` |  |
| `screen:torrents` | First | `g`, `Home` | `g`, `Home` | — |  |
| `screen:torrents` | Last | `G`, `End` | `G`, `End` | — |  |
| `screen:torrents` | Page down | `PgDn` | `PgDn` | — |  |
| `screen:torrents` | Page up | `PgUp` | `PgUp` | — |  |
| `screen:torrents` | Details | `Enter` | `Enter` | — |  |
| `screen:torrents` | Open | `o` | `o` | `o`, `Enter` | Popup: Enter opens the file (exception 1). |
| `screen:torrents` | Pause/resume | `p`, `Space` | `p`, `Space` | `p`, `Space` |  |
| `screen:torrents` | Seed on/off | `u` | `u` | `u` |  |
| `screen:torrents` | Remove, keep files | `d`, `x`, `Delete` | `d`, `x`, `Delete` | `d`, `x`, `Delete` |  |
| `screen:torrents` | Delete with files | `D` | `D` | — |  |
| `screen:torrents` | IMDb | `i` | `i` | — |  |
| `screen:torrents` | Wikipedia | `w` | `w` | — |  |
| `screen:torrents` | Follow show | `f`, `F` | `f` | — | Native TUI; legacy uses F while f is first/last priority. |
| `screen:torrents` | Follow show (legacy) | `F` | — | — |  |
| `screen:torrents` | Sequential (legacy) | `s` | — | — | Legacy qBittorrent only; native streaming priority is automatic. |
| `screen:torrents` | First/last priority (legacy) | `f` | — | — | Legacy qBittorrent only; native streaming priority is automatic. |
| `screen:search` | Sort results | — | `s` | — |  |
| `screen:search` | Reverse results sort | — | `S` | — |  |
| `pane:search.results` | Copy magnet | — | `y` | — |  |
| `screen:search` | Next | `j`, `↓` | `j`, `↓` | — |  |
| `screen:search` | Previous | `k`, `↑` | `k`, `↑` | — |  |
| `screen:search` | First | `g`, `Home` | `g`, `Home` | — |  |
| `screen:search` | Last | `G`, `End` | `G`, `End` | — |  |
| `screen:search` | Page down | `PgDn` | `PgDn` | — |  |
| `screen:search` | Page up | `PgUp` | `PgUp` | — |  |
| `screen:search` | Providers | `P` | `P` | — |  |
| `screen:search` | Browse lists | `b` | `b` | — |  |
| `screen:search` | Edit query | `/`, `e` | `/`, `e` | — |  |
| `screen:search` | Stop search | `Esc` | `Esc` | — |  |
| `pane:search.results` | Download | `Enter` | `Enter` | — |  |
| `pane:search.results` | IMDb | `i` | `i` | — |  |
| `pane:search.results` | Wikipedia | `w` | `w` | — |  |
| `pane:search.results` | Source page | `O` | `O` | — |  |
| `pane:search.results` | Follow show | `f`, `F` | `f`, `F` | — |  |
| `pane:search.query` | Search | `Enter` | `Enter` | — |  |
| `pane:search.query` | Leave query | `Esc`, `↓` | `Esc`, `↓` | — |  |
| `screen:rss` | Next | `j`, `↓` | `j`, `↓` | — |  |
| `screen:rss` | Previous | `k`, `↑` | `k`, `↑` | — |  |
| `screen:rss` | First | `g`, `Home` | `g`, `Home` | — |  |
| `screen:rss` | Last | `G`, `End` | `G`, `End` | — |  |
| `screen:rss` | Page down | `PgDn` | `PgDn` | — |  |
| `screen:rss` | Page up | `PgUp` | `PgUp` | — |  |
| `screen:rss` | Parent pane | `h`, `←` | `h`, `←` | — |  |
| `screen:rss` | Child pane | `l`, `→` | `l`, `→` | — |  |
| `screen:rss` | Open/download | `Enter` | `Enter` | — |  |
| `screen:rss` | New feed | `n` | `n` | — |  |
| `screen:rss` | Remove feed | `x` | `x` | — |  |
| `screen:rss` | Refresh | `r` | `r` | — |  |
| `screen:rss` | Reload saved feeds | `R` | `R` | — |  |
| `screen:rss` | Mark read/unread | `m`, `Space` | `m`, `Space` | — |  |
| `screen:rss` | All read | `M` | `M` | — |  |
| `screen:rss` | IMDb | `i` | `i` | — |  |
| `pane:rss.items` | Follow show | `f`, `F` | `f`, `F` | — |  |
| `screen:shows` | Next | `j`, `↓` | `j`, `↓` | — |  |
| `screen:shows` | Previous | `k`, `↑` | `k`, `↑` | — |  |
| `screen:shows` | First | `g`, `Home` | `g`, `Home` | — |  |
| `screen:shows` | Last | `G`, `End` | `G`, `End` | — |  |
| `screen:shows` | Page down | `PgDn` | `PgDn` | — |  |
| `screen:shows` | Page up | `PgUp` | `PgUp` | — |  |
| `screen:shows` | Parent pane | `h`, `←` | `h`, `←` | — |  |
| `screen:shows` | Child pane | `l`, `→` | `l`, `→` | — |  |
| `screen:shows` | Open/download | `Enter` | `Enter` | — |  |
| `screen:shows` | Follow show | `n` | `n` | — |  |
| `screen:shows` | Unfollow | `x` | `x` | — |  |
| `screen:shows` | Refresh | `r` | `r` | — |  |
| `screen:shows` | Check selected show | `R` | `R` | — |  |
| `screen:shows` | Mark read/unread | `m`, `Space` | `m`, `Space` | — |  |
| `screen:shows` | All read | `M` | `M` | — |  |
| `screen:shows` | IMDb | `i` | `i` | — |  |
| `screen:shows` | Wikipedia | `w` | `w` | — |  |
| `screen:shows` | EZTV | `O` | `O` | — |  |
| `screen:shows` | Undo read | `u` | `u` | — |  |
| `modal:files` | Close | `Esc`, `h`, `q`, `Enter` | — | — |  |
| `modal:files` | Next | `j`, `↓` | `j`, `↓` | — |  |
| `modal:files` | Previous | `k`, `↑` | `k`, `↑` | — |  |
| `modal:files` | Include/skip | `Space` | `Space` | — |  |
| `modal:files` | All | `a` | `a` | — |  |
| `modal:files` | None | `n` | `n` | — |  |
| `modal:files` | Reload | `r` | — | — |  |
| `modal:files` | First | `g`, `Home` | — | — |  |
| `modal:files` | Last | `G`, `End` | — | — |  |
| `modal:files` | Include | `1` | — | — |  |
| `modal:files` | High (legacy) | `2` | — | — |  |
| `modal:files` | Max (legacy) | `3` | — | — |  |
| `modal:providers` | Close | `Esc`, `q`, `Enter` | — | — |  |
| `modal:providers` | Next | `j`, `↓` | `j`, `↓` | — |  |
| `modal:providers` | Previous | `k`, `↑` | `k`, `↑` | — |  |
| `modal:providers` | Toggle | `Space` | `Space` | — |  |
| `modal:providers` | All | `a` | — | — |  |
| `modal:providers` | None | `n` | — | — |  |
| `modal:providers` | Reload | `r` | — | — |  |
| `modal:providers` | Open site | `o` | — | — |  |
| `modal:settings` | Cancel | `Esc` | — | — |  |
| `modal:settings` | Save | `Enter` | — | — |  |
| `modal:settings` | Next setting | `j`, `↓`, `Tab` | — | — |  |
| `modal:settings` | Previous setting | `k`, `↑`, `Shift+Tab` | — | — |  |
| `modal:settings` | Decrease | `h`, `←` | — | — |  |
| `modal:settings` | Increase | `l`, `→`, `Space` | — | — |  |
| `modal:help` | Close | `Esc`, `q`, `?` | — | — |  |
| `modal:help` | Scroll down | `j`, `↓` | — | — |  |
| `modal:help` | Scroll up | `k`, `↑` | — | — |  |
| `modal:help` | Page down | `PgDn` | — | — |  |
| `modal:help` | Page up | `PgUp` | — | — |  |
| `modal:help` | First | `g`, `Home` | — | — |  |
| `modal:help` | Last | `G`, `End` | — | — |  |
| `prompt` | Remove | `y` | `y` | `y` |  |
| `prompt` | Cancel | `Esc` | `Esc` | `Esc` |  |
| `prompt` | Submit | `Enter` | `Enter` | — |  |
| `prompt` | Sequential (legacy) | `F2` | — | — |  |
| `prompt` | First/last (legacy) | `F3` | — | — |  |
| `prompt` | Seed on/off | `F4` | — | — |  |
| `global` | Card | `v` | — | — |  |
| `modal:poster` | Back | `Esc`, `v` | — | — |  |
| `modal:poster` | IMDb | `i` | — | — |  |
| `modal:poster` | Wikipedia | `w` | — | — |  |
<!-- keys:end -->

The daemon checks its executable inode and modification time every 30 seconds.
An atomically installed replacement switches automatically when the busy table
is empty, through exit 75 and the systemd unit's forced restart. While work is
active, summary/app.info expose `upgrade_pending`; `restart` switches immediately.
Foreground `serve` exits 75 too, but requires its supervisor to launch a new owner;
CLI restart waits 30 seconds and exits 8 if no replacement hello arrives. Reads
and restart never start a closed daemon. Hello reports the compiled git build id.

Agents can use `fbittorrent events --follow --kinds torrent.completed --json`
instead of polling. Save `(instance, seq)` and pass `--since-seq` to replay; an
expired cursor emits an explicit resync marker. Ctrl+C detaches only the reader.
`fbittorrent api torrents.list --json` prints the unchanged API envelope. Raw
arguments accept repeated `--arg key=value` or one `--json-args` object; `--ref`
keeps an explicit mutation identity across invocations. `api --list --json` and
`robot-docs api` expose the operation catalogue. Reads never start a daemon.

Local integration consumers can run `python3
agent_ergonomics_audit/audit/regression_tests/daemon.py target/debug/fbittorrent
--serve-fixture downloading --throttle 32768` (also idle/seeding/paused; empty is
an idle alias). The FBTFIXTURE JSON line names its private socket, summary and
owned resources. The fixture stays alive until stopped; an initially closed
stdin does not stop it. Closing a pipe that was open at launch supports UI
teardown. A guardian removes both private peer daemons even after launcher
SIGKILL. --systemd uses only disposable transient units. Global throttle API
wiring follows P5-04; the current single-seed fixture uses its per-torrent cap.

Completed stop-on-complete records restore as `done, checked:false` without
opening their payloads for hashing. `seed` and nonempty file-selection actions
verify them before use; `open` only stats the saved target and stays dormant.
Missing data becomes `error` with `reason:missing_files` when acted on. Explicit
`delete --files --yes` can remove a dormant record without creating a live handle.


Private Jackett/Prowlarr indexers use native Torznab, alongside Nova providers:
`fbittorrent search --install-torznab https://indexer.example/api --key-file /path/to/indexer.key`.
Keep the key in its own user-owned 0600 regular file; indexers.toml has the same
requirement, and unsafe modes/symlinks are refused. Keyed HTTP is allowed only
on loopback. Queries are bounded to 30 seconds/4 MiB and refuse cross-origin
redirects; credentials are redacted from results and diagnostics. The providers
sheet/modal displays `nova` or `torznab`. See [IPC.md](docs/IPC.md#native-torznab-providers-p7-01-cli-contract-1226).
