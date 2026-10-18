# Changelog

The binary and the bar plugin share one version number. The API protocol and
the agent CLI contract (`fbittorrent capabilities --json`, `contract_version`)
are versioned separately.

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
