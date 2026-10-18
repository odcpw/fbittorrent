# Download queue, peer encryption and ProtonVPN binding

Completed and installed 2026-10-10. Final behavior checks, exact binary identity,
resource measurements and remaining evidence limits are recorded in
[the validation report](validation/2026-10-10/README.md). The terminal interaction
corrections and design research are recorded in
[the UX review](tui-ux-review.md).

## User outcome

- Run three downloads at a time by default. Additional downloads wait and start
  automatically when a slot opens. Manual pause remains a manual pause.
- Prefer encrypted peer connections, with an option to require encryption.
- Let the user bind torrent networking to ProtonVPN. If the connection is
  unavailable, keep the app usable and wait without sending torrent traffic
  through another connection.
- Keep all engine tasks within the open application's Asupersync lifetime.

## Implementation ownership

- Download queue agent: durable running intent, FIFO admission, queue state and
  live queue-limit API in the app engine.
- Peer encryption agent: standard MSE negotiation and encrypted streams for
  incoming, outgoing and magnet-metadata peers.
- VPN binding agent: kernel interface binding, bound DNS and HTTP/TLS, peer and
  discovery sockets, and cancellation when the interface becomes unavailable.
- Orchestrator: saved settings, ProtonVPN discovery, CLI and terminal controls,
  cross-feature integration, independent review and installation verification.

## Settings and presentation

- `max_active_downloads = 3`: positive integer; queue-only changes may apply live.
- `peer_encryption = "prefer"`: `off`, `prefer` or `require`.
- `network_interface`: absent means ordinary networking; `proton` follows the
  detected ProtonVPN interface; an interface name requires that exact interface.
- Show queued torrents separately from user-paused torrents. Show VPN waiting
  state separately from engine errors. Changes to encryption or binding drain
  and reopen the engine before new transfers begin.

## Verification

- Real local downloads exercise queue admission, completion, pause, removal,
  changed limits and reopening with more than three saved downloads.
- An independent BitTorrent implementation verifies encryption in both
  directions, metadata and payload, fallback and required-encryption refusal.
- Real bound sockets and a disposable network environment exercise IPv4/IPv6,
  DNS, trackers, incoming peers, interface loss and no ordinary-interface fallback.
- Check cancellation, app close, dependency graph, terminal settings controls,
  and unchanged RSS/Shows/file selection. Measure startup, idle use and shutdown
  against the previously installed binary; do not claim unmeasured gains.

## Boundaries

No changes to season-pack file selection, fast resume, router forwarding, uTP
or BitTorrent v2. No background daemon, VPN connection changes or real user
download fixtures. Preserve the existing dirty checkout and installation backup.
