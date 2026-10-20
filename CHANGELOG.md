# Changelog

The binary and the bar plugin share one version number. The API protocol and
the agent CLI contract (`fbittorrent capabilities --json`, `contract_version`)
are versioned separately.

## v0.6.0 (2026-10-20)

### One app, one daemon

fbittorrent now runs an on-demand daemon with an embedded Rust torrent engine.
The Omarchy bar, popup, dashboard, TUI and CLI share one local API and see the
same transfers. Windows attach and detach independently; saved downloads survive
restarts and resume when a required network interface returns.

### Notable features

- Review magnets, torrent URLs and local `.torrent` files in a staged add sheet;
  choose the destination and files, then explicitly Download or Discard.
- Search, RSS and Shows run in the daemon, with shared results, read markers,
  a Shows inbox, quality preferences and Torznab catalogues. Checking a feed or
  show never starts a download automatically.
- Ready-to-watch feedback uses verified video bytes and a smoothed rate estimate
  to distinguish readiness from playback that may stall. MP4/MOV also require
  the complete `moov` metadata, including when it sits at the end of the file.
  ([Playback checks](https://github.com/odcpw/fbittorrent/commit/bc9ccdf2b877884bb7aec8540df682114db3b129))
- Turtle limits, per-torrent download caps, queue controls and asynchronous recheck.
  Details expose Files, Peers, Trackers, Pieces and Log; trackers can be edited.
- Move downloads with a folder picker, rename their display titles without
  renaming files, and Undo a keep-files removal within ten seconds.
- A quick palette opens add/search directly. Opt-in clipboard watching offers
  magnets for review; it never commits a download by itself.

### Defaults and controls

- **Zero payload upload by default**, during downloading and after completion.
  Upload needs explicit permission; completed torrents require per-torrent opt-in.
- **Stop rule:** the bar icon is lit exactly while the daemon runs. Closing a UI
  leaves transfers running; explicit Stop, `down` or uppercase Q drains the daemon
  and transfers. Idle exit stops an unused daemon after five minutes by default.
- No backwards compatibility: this release uses today's native data format and
  API throughout. Existing current-format configuration, RSS, Shows and torrent
  records continue to load.

### Security hardening

- Private local sockets and stores, bounded API frames and provider processes,
  redacted feed/tracker credentials, protected Torznab keys, and safe handling of
  payload paths, file deletion, terminal text and links. Configured interface
  binding governs all app traffic, including RSS/Shows, IMDb and Nova search.
  Python providers use an authenticated local SOCKS5h proxy with interface-bound
  DNS and TCP; an unavailable tunnel fails closed without a normal-route fallback.
  ([Input and credential hardening](https://github.com/odcpw/fbittorrent/commit/f5d17cd42a9a5bcd7c2e08d694630ce447f59ba6),
  [bound outbound traffic](https://github.com/odcpw/fbittorrent/commit/d5f2c4ecaf16eba24263ac803ad5083259dbd0a9))

### Reliability

- Ordered shutdown drains within its deadline under disk pressure: durable
  checkpoint syncs are cancellable, workers release the engine before joining,
  and completed payloads are certified before a dormant restore.
- Runtime regions are admitted by memory pressure rather than a fixed count,
  so 500 active torrents start.
- Peers that only fetch metadata stay choked, so zero upload holds at magnet
  resolution too.
- `fbittorrent setup` installs the bar plugin from files embedded in the binary,
  keeps one bar entry, and restarts the shell so the widget matches the daemon.

### Performance

- Pushed state and socket wakeups replace idle UI polling. Subscribers share
  sampled summaries, paused tables stay quiet, and one typed torrent model reaches
  the API and TUI without a JSON conversion round trip. Startup and shutdown do
  less background work.
  ([Shared summaries](https://github.com/odcpw/fbittorrent/commit/b534ab6aa449f5d2d0a14a9591f496bc7722ed76),
  [socket wakeups](https://github.com/odcpw/fbittorrent/commit/7bfd93c5a4537a8539d018739bae6d132ec348c0))

### Removed

- qBittorrent WebUI mode, the legacy socket adapter, and the qBittorrent-shaped
  model and compatibility layer. The daemon owns every native transfer.

## v0.1.1 (2026-10-18)

The native product lands on `main`. Until this release `main` held the
September 2026 app, a client for a local qBittorrent-nox WebUI with a
three-screen TUI; the native engine existed only as an uncommitted working
tree.

### Added

- Embedded torrent engine: the rqbit 9.0.1 crate family, vendored in
  `vendor/rqbit/` and ported to the Asupersync 0.5.0 runtime (`rqbit-runtime`
  replaces Tokio and reqwest across the engine, DHT, trackers, discovery and
  storage). Sequential download with first/last piece priority is always on.
- Seeding off by default: a 32 KiB/s upload cap while downloading and stop on
  completion, with a per-torrent `seed` override (`u` in the TUI and widget,
  `fbittorrent seed <hash>`).
- Download queue (three active downloads by default, F6 to change), peer
  encryption (MSE, prefer/require/off) and Linux interface binding for
  ProtonVPN (`proton0`), which stops transfers while the tunnel is down.
- An app-lifetime local transport between the open app and the CLI (also the
  `magnet:` handler) and the bar widget (`src/ipc.rs`), and the app controls
  and network-waiting state around the engine (`src/network_app.rs`).
- Shows: a local watchlist with EZTV episodes and season packs, an "All
  unread" inbox, read markers and download history; checking never downloads.
- RSS feeds with a local read history, and search through qBittorrent's nova3
  runtime (`vendor/nova3/`, embedded) with streamed provider results.
- TUI screens Torrents, Search, Shows and RSS, with IMDb cards, mouse hover,
  click and wheel support, file selection and scrollable help.
- Tests: native integration (`native.py`, isolated XDG directories and a real
  tmux TUI), engine benchmarks and close/resource checks under
  `agent_ergonomics_audit/audit/regression_tests/`; repository hygiene checks
  under `scripts/` (run by `all.sh`).
- Docs: engine evaluation, queue/encryption/VPN plan, TUI review, the
  2026-10-18 plan with its beads (`.beads/`), and the validation receipt
  summaries (`docs/validation/**/README.md`).
- `LICENSE` (MIT) and `NOTICE` naming the vendored licences.

### Changed

- The bar widget (`plugin/oliver.torrents/`) talks to the native app instead
  of the qBittorrent WebUI.
- Asupersync 0.5.0 is patched and vendored in full (`vendor/asupersync/`,
  selected through `[patch.crates-io]`): a timer-wheel `next_deadline` cleanup
  that removes repeated scans of cancelled timeouts, the measurement-only
  `epoch-diagnostics` feature, and the region-owner key the engine uses
  (`vendor/asupersync/FBITTORRENT_PATCH.md` lists the ten files).

### Notes

- `Cargo.toml` and `plugin/oliver.torrents/manifest.json` still read version
  0.1.0; the `v0.1.1` tag marks this landing.
- The validation receipt data of 2026-10-09 to 2026-10-14 (about 279 MB) was
  deleted; only the README summaries are kept (owner decision Q4).
