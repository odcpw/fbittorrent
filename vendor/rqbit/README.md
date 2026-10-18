# rqbit components used by fbittorrent

The vendored protocol and engine source comes from the published `librqbit`
9.0.1 crate family. Its recorded upstream commit is
`a499d2f243d124e144aef137afe7cb304a6e3f36` in
<https://github.com/ikatson/rqbit>. `librqbit-dualstack-sockets` 0.7.0 records
commit `e2f221ca745c25c7790abb593ed260ce5a499fa1`. Published crate metadata and
`.cargo_vcs_info.json` are retained. Upstream copyright and Apache 2.0 terms are
in `LICENSE`; the socket crate also retains its own `LICENSE`.

## Local changes

fbittorrent ports the used engine surface to Asupersync 0.5.0. The local
`rqbit-runtime` crate supplies native contexts, child regions, tasks, timers,
channels, synchronization, TCP/UDP, HTTP/TLS, and bounded blocking work. These
replace Tokio and reqwest across the engine, DHT, trackers, discovery, and
storage. Task factories receive the owning context, and pause/close paths
cancel and drain their regions. The app owns durable torrent metadata and
records, rather than rqbit's upstream persistence implementation.

`ManagedTorrent::have_pieces_snapshot()` returns an owned, in-memory
`Option<Box<dyn BitV>>` for watch-readiness and piece displays; `BitV` is exported
at the crate root. The bitfield is copied while the existing tracker read locks
are held and keeps the protocol's MSB-first byte order and final-byte padding.
Use metadata's piece count to interpret the meaningful bits. Initializing or
tracker-less states return `None`; initialized paused torrents retain their
verified bitfield. Snapshots can be mutated or dropped independently and never
flush or modify the source tracker. Native unit tests cover verified live bits,
paused/absent trackers, clone independence and a 10,000-piece clone below 1 ms.

The app uses TCP peer transport, DHT, HTTP/HTTPS and UDP trackers, local peer
discovery, file inclusion, sequential piece scheduling, and upload limits.
The maintained engine also supports MSE peer encryption in both directions,
including magnet metadata, with off/prefer/require policies. Its adapted
protocol and license attribution live in `librqbit/src/mse/README.md`.
Linux interface binding covers engine TCP/UDP, trackers, discovery and
HTTP/TLS/DNS; required-device failure never retries on an unbound socket.
Web serving, UPnP, uTP, SOCKS, PostgreSQL, and upstream fast resume are outside
this integration. Unsupported session options are rejected where exposed.

Upstream Tokio integration tests are retained as source references but are
not compiled. Native module tests and fbittorrent's engine tests exercise the
ported behavior. See `docs/engine-evaluation.md` at the repository root for
the tested scope and remaining limits. This is a maintained source fork;
upstream changes require deliberate review and rerunning native tests.
