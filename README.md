# fbittorrent

A small torrent app for Omarchy, with an embedded Rust engine, a native bar
widget, a FrankenTUI app, and a CLI that doubles as the `magnet:` handler.
The engine runs in the open app; closing it stops transfers, peer discovery,
search jobs, and feed checks. Torrent records and feed history stay on disk.

- **Streaming priority is automatic.** The native engine downloads each file
  sequentially and prioritizes its first and last pieces. These policies cannot
  be toggled off in native mode.
- **Seeding off by default.** New torrents get a 32 KiB/s upload cap while
  downloading and stop when complete. This limits uploaded payload; it does not
  guarantee zero upload. Allow seeding per torrent with `u` in the TUI or widget,
  or `fbittorrent seed <hash>`; that removes the cap and allows completed torrents
  to keep running.
  Flip the default with `seed_by_default = true` in the config.
- **App lifetime.** CLI actions open or focus the app when they need its engine.
  Widget torrent actions require an already open app. Explicit Open TUI actions
  may open it. Status queries never open it. Nothing runs at login. Closing the
  terminal or quitting the TUI stops the native engine.
  Terminal hangups drain the engine and remove its socket before exiting with
  the signal status (`128 + signal`, so SIGHUP exits 129). For diagnosis, set
  `FBT_DEBUG_SIGNALS=1`: lifecycle events are appended to
  `$XDG_STATE_HOME/fbittorrent/tui-signals.log` (normally `~/.local/state/...`).
- **Bar widget** with the same chrome as the Wi-Fi panel: rates, progress rows,
  pause/remove, add from clipboard, one click to the TUI. It only shows while the
  app is open. Hover clipped names or details to see their full text.
- **TUI** with four screens: (1) Torrents, (2) Search (streamed provider results),
  (3) Shows (your local show list and releases), and (4) RSS (feeds and items).
  IMDb card per title:
  rating, blurb, poster; `i` opens IMDb, `w` opens the Wikipedia article. Hover
  a row to preview its full name; click selects it, double-click activates it,
  and the wheel moves the list under the pointer. Hover leaves the keyboard
  target unchanged. Long hovered or selected names scroll after a short pause.
  The bottom rows show contextual commands such as `(O)pen`, with `Shift`
  spelled out where needed, and separate action feedback.
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
cargo build --release && cp target/release/fbittorrent ~/.local/bin/
fbittorrent setup --magnet          # app config, download folder, magnet: handler
fbittorrent search --install        # provider scripts; Python 3 is required
ln -s "$PWD/plugin/oliver.torrents" ~/.config/omarchy/plugins/oliver.torrents
omarchy plugin enable oliver.torrents
omarchy bar put oliver.torrents --before omarchy.tailscale
```

Menu entries live in `~/.config/omarchy/extensions/omarchy-menu.jsonc` under
`torrents.*`.

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
fbittorrent up | down                open/focus / close the app and engine
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
readiness datagrams. `--idle-exit-secs N` defaults to 300 (0 means disabled) and
is exposed in daemon policy metadata; the automatic idle policy and desktop
start-on-demand integration follow in the next implementation stage. The current
core runs until stopped. Daemon exits 70 (panic) and 75 (upgrade) belong to the
unit, outside the CLI's 0–8 dictionary. API v1 and TUI attachment are subsequent
stages; the current CLI uses the existing framed socket.

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
starting the app or legacy service. Commands that need a closed engine return
exit 4 with an opening hint. This also prevents explicit `up` and `tui` startup;
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

`python3 scripts/dagcheck.py` and `python3 scripts/mockwidth.py` review the plan's
dependency graph and screen mockups. They run by hand, outside CI.

```
agent_ergonomics_audit/audit/regression_tests/all.sh   # default isolated gate; live suites below require opt-in
cargo test --locked -- --nocapture                    # engine records, feeds/shows/search, TUI render/interactions, legacy client
python3 agent_ergonomics_audit/audit/regression_tests/native.py  # isolated native app + real CLI/TUI actions
…/regression_tests/run.sh                              # CLI agent contract (exit codes, JSON, safety)
…/regression_tests/tui_smoke.sh                        # drives the TUI in tmux: list, files modal, prompt toggles, screens, help, quit
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
