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

P6-01 keeps `TrackerStats` on `ManagedTorrentShared`, outside run regions.
`tracker_stats()` returns URL-sorted owned observations through pause;
`announce_now()` wakes HTTP/UDP interval and retry timers with event=none,
coalescing forced announces for sixty seconds. Unsupported schemes and private
secondary URLs remain visible as not_contacted. Tracker diagnostics strip URL
queries, including remote error text, before logging or surfacing messages.
Handle replacement during P7-02 moves transfers the serde-skipped, app-only
`AddTorrentOptions::tracker_stats` Arc, preserving observation history and the
monotonic forced-announcement limit across relocation.
Real local HTTP/UDP tracker tests cover failure, forced success, rate limiting
and pause retention; the existing private-torrent discovery rules stay intact.

P6-06 adds a serde-skipped, app-only `AddTorrentOptions::trusted_have_pieces`.
The initializer consumes it once, validates byte-rounded length and zero
padding, and builds the normal chunk tracker without checksum reads. The app
owns the clean marker, file-stamp/selection/checksum checks and cache invalidation;
upstream persistence and fastresume remain rejected. Later error recovery never
reuses admission-time trusted bits. See the fast-resume section in the engine
evaluation for the trust boundary and physical-read regression evidence.

The app uses TCP peer transport, DHT, HTTP/HTTPS and UDP trackers, local peer
discovery, file inclusion, sequential piece scheduling, and upload limits.
P6-07 adds `ManagedTorrent::set_upload_bps(Option<u64>)` for synchronous
per-torrent cap changes without re-admission or hashing. The limiter belongs
to the shared torrent configuration, so initialization and pause/resume use
the current cap. `None` means unlimited; finite caps are 16 KiB/s through
`u32::MAX`. Waiting payload requests re-read a replaced limiter. App-owned
ledger records preserve the cap across process restarts.
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

### Unused adapter cleanup (2026-10-20)

The app uses its own typed API model. Removed the unused bitfield-load hook and
unused listener result field; the disk-snapshot durability regression lives in
test-only support because production restore always verifies content. Metadata
resolution still retains the peer receiver until its result is consumed. No
transfer, shutdown or storage ordering changes.
