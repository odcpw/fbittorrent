# fbittorrent

A small, fast torrent client for Omarchy. qBittorrent does the heavy lifting in the
background; fbittorrent is the part you actually see: a native bar widget, a
FrankenTUI app, and a CLI that doubles as the `magnet:` handler.

- **Sequential download + first/last piece priority on by default** for every add.
- **No upload by default.** New torrents get a 32 KiB/s upload cap (just enough for
  protocol chatter; libtorrent throttles piece requests against it, so a 1 B/s cap
  would stall downloads) and stop the moment they complete. Allow seeding per torrent with `u` in the TUI or widget, or
  `fbittorrent seed <hash>`; that tags it `seed` so the reconciler leaves it alone.
  Flip the default with `seed_by_default = true` in the config.
- **On-demand backend.** qbittorrent-nox runs as a systemd user service that starts
  when you use fbittorrent and stops when you tell it to. Nothing runs at login.
- **Bar widget** with the same chrome as the Wi-Fi panel: rates, progress rows,
  pause/remove, add from clipboard, one click to the TUI. It only shows while the
  backend is running.
- **TUI** with three screens: Torrents, Search (17 providers via qBittorrent's
  search plugins, streamed), RSS (add feeds, grab items). IMDb card per title:
  rating, blurb, poster; `i` opens IMDb, `w` opens the Wikipedia article.
- **Provider browser.** Enter on an empty search box (or `b`) lists: 1337x
  (trending, top movies, top TV, opened in the browser), YTS latest, EZTV latest
  episodes, and The Pirate Bay top-100 movies and TV. Lists that load in the TUI
  carry IMDb ids, so the poster card appears as soon as the cursor rests on a
  title. Hover lookup also works on the Torrents screen and on RSS items.
- **Pick your providers.** `s` on the Search screen opens a checkbox list of the
  installed search plugins; the choice is stored in qBittorrent and applies to
  every search. Results show an UPLOADED column where the provider reports one,
  and an IMDB column with the rating: instant for sources that carry IMDb ids
  (browse lists), filled in for others once their title lookup completes.
  Duplicate torrents from several providers are collapsed by info hash.
- **File selection.** Enter on a torrent opens its file list: Space toggles a
  file, `1`/`2`/`3` set normal/high/max priority, `a`/`n` select all or none.
- **Themed** from the active Omarchy theme, live, no template needed.

## Install

```bash
omarchy pkg add qbittorrent-nox
cargo build --release && cp target/release/fbittorrent ~/.local/bin/
fbittorrent setup --magnet          # config, user unit, magnet: handler
fbittorrent search --install        # official search plugins
ln -s "$PWD/plugin/oliver.torrents" ~/.config/omarchy/plugins/oliver.torrents
omarchy plugin enable oliver.torrents
omarchy bar put oliver.torrents --before omarchy.tailscale
```

Menu entries live in `~/.config/omarchy/extensions/omarchy-menu.jsonc` under
`torrents.*`.

## CLI

```
fbittorrent                          open the TUI (starts the backend if needed)
fbittorrent tui [--search|--rss]     same, starting on a given screen
fbittorrent add <magnet|url>         add with default flags, desktop notification
fbittorrent add-clipboard            add whatever magnet/URL is on the clipboard
fbittorrent status [--json]          rates and counts; never starts the backend
fbittorrent list [--active]          torrents with hash prefix, %, speed, eta, S/F flags
fbittorrent start|stop|delete <hash-prefix>...   (delete --files removes data)
fbittorrent seq|first-last <hash>    toggle flags
fbittorrent seed|noseed <hash>...    allow / forbid uploading for a torrent (cap = 32 KiB/s)
fbittorrent reconcile [--recent N]   enforce default flags on torrents added elsewhere
fbittorrent rss list|add|remove|refresh|items|grab
fbittorrent search <query> [--wait S]
fbittorrent search --install         official + curated community providers (17 total)
fbittorrent search --install-url <url>   any qBittorrent search plugin (.py) from a URL
fbittorrent search --plugins         list installed providers
fbittorrent imdb <release name>      rating, blurb, poster path
fbittorrent up | down                start / stop qbittorrent-nox
fbittorrent config                   effective config (~/.config/fbittorrent/config.toml)
```

## Config

`~/.config/fbittorrent/config.toml`

```toml
url = "http://127.0.0.1:8080"
sequential = true
first_last = true
seed_by_default = false
notify = true
timeout_secs = 10
autostart = true
backend_unit = "qbittorrent-nox.service"
# save_path = "/home/you/Downloads"      # qBittorrent default set by `setup`
# category = "movies"
# username / password only if you enabled WebUI auth for localhost
```

## Layout

```
src/main.rs        CLI (clap)
src/qbt.rs         qBittorrent WebUI API v2 client (ureq, cookie session)
src/backend.rs     start/stop the user service on demand
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
torrentdownloads, kickasstorrents, snowfl, zooqle, eztvx, nyaa). Add any other
plugin from <https://github.com/qbittorrent/search-plugins/wiki/Unofficial-search-plugins>
with `--install-url`. Private trackers usually need Jackett or Prowlarr; the
official `jackett.py` plugin works once you have one of those running.

## Why some providers are browser-only

1337x answers every script with a Cloudflare challenge page, and YTS's main
domain does not resolve from this network (its mirrors redirect back to it), so
those entries open the site in your browser instead of loading a list. The
Pirate Bay (`apibay.org`) and EZTV publish plain JSON and work in the TUI.

## Notes

- IMDb's title pages are bot-gated, so ratings come from IMDb's official daily
  ratings dataset (cached in `~/.cache/fbittorrent`, refreshed after 24h) and the
  blurb from Wikipedia's summary API. Posters are IMDb's own CDN images.
- Search jobs in qBittorrent are bound to the WebUI session cookie; the client keeps
  one session per process for that reason.
- Personal use is legal in Switzerland. Mind your local law elsewhere.
