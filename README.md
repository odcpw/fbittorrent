# fbittorrent

fbittorrent is a torrent app for Omarchy: an on-demand daemon with an embedded
Rust engine, a bar widget and popup, a dashboard, a terminal UI and a CLI.
The daemon owns transfers; each interface connects to the same local API.
Open the dashboard to search, review a download, watch its progress, manage
Shows/RSS and inspect Files, Peers, Trackers, Pieces or Log in Details.

## Install

Requires Omarchy with Quickshell and a systemd user session. Building uses the
pinned Rust toolchain; search providers need Python 3, and clipboard features
need `wl-clipboard`.

From this checkout:

```sh
scripts/install.sh                       # build and install in ~/.local/bin
scripts/install.sh --plugin copy         # copy the plugin rather than link it
```

Or put a built `fbittorrent` binary on PATH and install its desktop integration:

```sh
fbittorrent setup --magnet --plugin=copy
```

The binary includes both plugins; copy installs need no source checkout.
`--plugin=link` links a checkout found from the working directory or binary
location, falling back to an embedded copy when neither is in a checkout. Setup installs
the on-demand user service, magnet/`.torrent` handlers, icon, menu and bar entry.
Changed plugin installs **restart the Omarchy shell**, recreating open windows
with the new code. An unchanged repeat is a no-op. Nothing is enabled at login.
`scripts/install.sh --binary /path/to/fbittorrent` installs an existing build.

```sh
fbittorrent search --install             # install the Python search providers
fbittorrent dashboard                    # open the desktop window
fbittorrent tui                          # open the TUI in this terminal
```

Search providers are local Python programs; install sources you trust.
`fbittorrent setup --uninstall` removes integration and keeps saved data and downloads.

## Stop rule

**The bar icon is lit exactly while the daemon runs.** The popup power button,
`fbittorrent down`, or uppercase **Q** stops the daemon and all transfers. The
UIs ask for confirmation when transfers are active; `down` stops directly.
Closing a UI window, lowercase **q** or Ctrl+C in the UI never stops transfers.
By default the daemon exits after five idle minutes: transfers, interactive
windows, searches and pending operations keep it alive. The bar alone does not;
an open popup pauses the countdown. Set `idle_exit_secs = 0` to disable idle exit.

## Downloads and upload permission

Dashboard and browser adds open a review sheet for magnets, torrent URLs and
local `.torrent` files.
Inspect the destination and files; **Enter / Download** commits once, **Esc /
Discard** removes the stage and keeps files. The sheet opens before metadata;
Download can proceed while the file list is still being fetched. The default
browser handler uses this review (`handler = "confirm"`); `handler = "add"` adds
immediately. Ordinary CLI `add` also adds directly.

**Upload is off by default** (`upload = "off"`): no piece payload is uploaded
while downloading or after completion. The explicit **Upload: off/on** toggle in
Details permits upload for that torrent, including after completion; switching
it off persists that torrent's opt-out. Global Upload in Settings, or
`fbittorrent settings --upload on`, permits upload during downloads. Completed
torrents still stop unless individually opted in. CLI `add --upload` explicitly opts in when adding; the dashboard uses the Details toggle.

Streaming priority is automatic. A verified video prefix produces **Ready to
watch** or **Playable · may stall** according to the known buffer and transfer
rate. **o** opens it in the configured player.
Checking Shows or RSS never downloads a release automatically; review and
Download are deliberate, and inbox read state is separate from transfer state.

## Keys and everyday use

**1–4** select Torrents / Search / Shows / RSS; **Tab / Shift+Tab** cycle them.
**j/k** or arrows move, **Enter** activates, **/** edits the query or filter,
**?** opens contextual help, **,** opens Settings (**F6** also works in the TUI).
In Torrents, **Enter** opens Details, **[ / ]** change its tab, **p / Space**
pause/resume, **u** opens Upload details, **d** removes while keeping files and
**D** explicitly deletes files. Confirmation starts on Cancel. In dashboard
Torrents, **U** undoes a keep-files removal within its ten-second window.
**z** toggles turtle limits. Popup **w** opens the dashboard; **t** opens the TUI.

The [M2 demo](docs/M2-demo.md) walks through search → review → watch, the Shows
inbox and cleanup. [Owner checks](docs/OWNER-CHECKS.md) collect the remaining
human checks. The full key reference below is generated from the action
catalogue, also available as [keymap.json](plugin/oliver.torrents/keymap.json) or
`fbittorrent robot-docs keys --surface all`.

<details>
<summary>Full generated keymap: TUI, dashboard and popup</summary>

<!-- keys:begin -->
| Scope | Action | tui | dashboard | popup | Note |
| --- | --- | --- | --- | --- | --- |
| `modal:clipboard` | Add offered magnet | — | `y` | `y` |  |
| `modal:clipboard` | Dismiss offered magnet | — | `Esc` | `Esc` |  |
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
| `modal:clipboard` | Add | `y` | — | — |  |
| `modal:clipboard` | Dismiss | `Esc` | — | — |  |
| `modal:stopped` | Start fbittorrent | `Enter` | — | — |  |
| `modal:details` | Rename | `N` | — | — |  |
| `modal:details` | Add | `+` | `+` | — |  |
| `modal:details` | Remove | `x` | `x` | — |  |
| `modal:details` | Reannounce | `e` | `e` | — |  |
| `modal:details` | Refresh | `r` | — | — |  |
| `modal:details` | Next tracker | `↓`, `j` | — | — |  |
| `modal:details` | Previous tracker | `↑`, `k` | — | — |  |
| `modal:details` | Next peer | `↓`, `j` | — | — |  |
| `modal:details` | Previous peer | `↑`, `k` | — | — |  |
| `modal:details` | Back | `Esc`, `h` | — | — |  |
| `modal:details` | Next tab | `]` | — | — |  |
| `modal:details` | Previous tab | `[` | — | — |  |
| `modal:files` | Next tab | `]` | — | — |  |
| `modal:files` | Overview | `[` | — | — |  |
| `modal:details` | Move… | `m` | `m` | — |  |
| `modal:details` | Download cap… | `c` | `c` | — |  |
| `modal:details` | Open | `o` | — | — |  |
| `modal:details` | Pause/resume | `p` | — | — |  |
| `modal:details` | Upload on/off | `u` | `u` | — |  |
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
| `screen:torrents` | Copy magnet | `y` | `y` | — |  |
| `screen:torrents` | Rename | `N` | `N` | — |  |
| `screen:torrents` | Undo removal | `U` | `U` | — |  |
| `screen:torrents` | Sort | `s` | `s` | — |  |
| `screen:torrents` | Reverse sort | `S` | `S` | — |  |
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
| `screen:torrents` | Upload details | `u` | `u` | `u` |  |
| `screen:torrents` | Remove, keep files | `d`, `x`, `Delete` | `d`, `x`, `Delete` | `d`, `x`, `Delete` |  |
| `screen:torrents` | Delete with files | `D` | `D` | — |  |
| `screen:torrents` | IMDb | `i` | `i` | — |  |
| `screen:torrents` | Wikipedia | `w` | `w` | — |  |
| `screen:torrents` | Follow show | `f`, `F` | `f` | — |  |
| `screen:torrents` | Select all visible | — | `Ctrl+A` | — |  |
| `screen:torrents` | Filter torrents | — | `/` | — |  |
| `screen:torrents` | Parent pane | — | `h`, `←` | — |  |
| `screen:torrents` | Child pane | — | `l`, `→` | — |  |
| `screen:search` | Browse grid/rows | — | `v` | — | Browse lists only. |
| `screen:search` | Sort results | `s` | `s` | — |  |
| `screen:search` | Reverse results sort | `S` | `S` | — |  |
| `pane:search.results` | Copy magnet | `y` | `y` | — |  |
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
| `pane:rss.items` | Sort items | `s` | — | — |  |
| `pane:rss.items` | Reverse item sort | `S` | — | — |  |
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
| `screen:rss` | Remove feed | `x` | `x`, `d` | — |  |
| `screen:rss` | Refresh | `r` | `r` | — |  |
| `screen:rss` | Reload saved feeds | `R` | — | — |  |
| `screen:rss` | Mark read/unread | `m`, `Space` | `m`, `Space` | — |  |
| `screen:rss` | All read | `M` | `M` | — |  |
| `screen:rss` | IMDb | `i` | — | — |  |
| `pane:rss.items` | Follow show | `f`, `F` | — | — |  |
| `screen:rss` | Refresh all feeds | — | `R` | — |  |
| `screen:rss` | Toggle unread filter | — | `u` | — |  |
| `screen:shows` | Sort releases | `s` | — | — |  |
| `screen:shows` | Reverse releases sort | `S` | — | — |  |
| `pane:shows.releases` | Copy magnet | `y` | — | — |  |
| `screen:shows` | Show quality | `=` | `=` | — |  |
| `screen:shows` | Inbox filters | `F` | `F` | — |  |
| `screen:shows` | Next | `j`, `↓` | `j`, `↓` | — |  |
| `screen:shows` | Previous | `k`, `↑` | `k`, `↑` | — |  |
| `screen:shows` | First | `g`, `Home` | `g`, `Home` | — |  |
| `screen:shows` | Last | `G`, `End` | `G`, `End` | — |  |
| `screen:shows` | Page down | `PgDn` | `PgDn` | — |  |
| `screen:shows` | Page up | `PgUp` | `PgUp` | — |  |
| `screen:shows` | Parent pane | `h`, `←` | `h`, `←` | — |  |
| `screen:shows` | Child pane | `l`, `→` | `l`, `→` | — |  |
| `screen:shows` | Open/download | `Enter` | `Enter` | — |  |
| `screen:shows` | Follow show | `n` | `n`, `+` | — |  |
| `screen:shows` | Unfollow | `x` | `x`, `d` | — |  |
| `screen:shows` | Refresh | `r` | `r` | — |  |
| `screen:shows` | Check selected show | `R` | `R` | — |  |
| `screen:shows` | Mark read/unread | `m`, `Space` | `m`, `Space` | — |  |
| `screen:shows` | All read | `M` | `M` | — |  |
| `screen:shows` | IMDb | `i` | — | — |  |
| `screen:shows` | Wikipedia | `w` | — | — |  |
| `screen:shows` | EZTV | `O` | — | — |  |
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
| `global` | Card | `v` | — | — |  |
| `modal:poster` | Back | `Esc`, `v` | — | — |  |
| `modal:poster` | IMDb | `i` | — | — |  |
| `modal:poster` | Wikipedia | `w` | — | — |  |
<!-- keys:end -->

</details>

## Settings and data

Use dashboard Settings, TUI **F6 / ,**, or `fbittorrent settings` for live changes.
`fbittorrent config` shows the effective configuration and its path. Defaults:

```toml
upload = "off"
idle_exit_secs = 300
max_active_downloads = 3
peer_encryption = "prefer"
handler = "confirm"
clipboard_watch = false
# save_path = "/home/you/Downloads"
# network_interface = "proton"
```

A required network interface stops torrent traffic when unavailable and resumes
saved download intent when it returns. Search, RSS and title lookups use the
usual connection. Turtle limits are separate from upload permission.

| What | Default location |
|---|---|
| Configuration | `~/.config/fbittorrent/config.toml` |
| Torrent records and metadata | `~/.local/share/fbittorrent/engine/` |
| Shows, RSS, search preferences and playback acknowledgements | `~/.local/share/fbittorrent/` |
| Statistics totals and UI preferences | `~/.local/state/fbittorrent/` |
| IMDb data, ratings and posters | `~/.cache/fbittorrent/` |
| Local API socket and instance lock | `$XDG_RUNTIME_DIR/fbittorrent/` |
| Downloaded files | Your Downloads directory, or the chosen `save_path` |

The corresponding `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME` and
`XDG_CACHE_HOME` override those defaults. Downloaded files are separate from
application data; Remove keeps them, Delete files requires explicit confirmation.
Logs: `fbittorrent log --follow` or `journalctl --user -u fbittorrent`.

## CLI and agents

```sh
fbittorrent search "Sintel" --json
fbittorrent status --json
fbittorrent list --json
fbittorrent shows list --json
fbittorrent rss list --json
fbittorrent robot-docs
```

The CLI works without a desktop; the TUI also works over SSH. Commands needing
the engine start the daemon on demand. Reads never start it; `--no-start` or
`FBITTORRENT_NO_START=1` prevents startup. Without a user manager, run
`fbittorrent serve` in a terminal or tmux. Select torrents by hash prefix, unique
name text or `all`; ordinary CLI commands accept `--json`.

Agents: start with `fbittorrent robot-docs`, then `triage --json` for current
state and next actions. `capabilities`, `robot-docs api` and `api --list --json`
describe commands, exit codes and API operations. `events --follow --json`
streams updates; `wait HASH --until playable --json` waits for playback readiness.
See [IPC.md](docs/IPC.md) for the local API contract. Local verification uses
`scripts/gate-fast.sh`; integration details live in [plugin/tests](plugin/tests/README.md).
Hosted GitHub Actions is disabled by default.
