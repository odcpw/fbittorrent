# Embedded engine: implementation and checked scope

## Choice

fbittorrent uses the **rqbit 9.0.1 protocol and engine components ported to
Asupersync 0.5.0**. rqbit was the simpler fit for the existing client contract:
its embedded session already supplied magnets, torrent files, file selection,
pause/resume, statistics and limits. The port retains its bencode, SHA1, piece,
and peer-protocol algorithms. The maintained fork is in `vendor/rqbit`.

IronTide was considered; comparative throughput and memory remain unmeasured.
The choice is based on integration fit. rqbit's published family is Apache 2.0;
its provenance and local changes are recorded in `vendor/rqbit/README.md` and
`LICENSE`. [Pinned upstream source](https://github.com/ikatson/rqbit/tree/a499d2f243d124e144aef137afe7cb304a6e3f36),
[librqbit API](https://docs.rs/librqbit/9.0.1/librqbit/).

An earlier prototype used upstream Tokio. Running it without Tokio failed even
with discovery disabled, so removing an app dependency was insufficient. The
current fork replaces the used runtime surfaces across the engine, DHT,
trackers, local discovery, sockets, HTTP and storage. The saved normal dependency
graph has no Tokio, reqwest or hyper. Test-only dependencies may contain Tokio;
upstream integration tests and examples that require it are retained as source
references and excluded from this app's build.

## Ownership and durable state

- `NativeRuntime` creates two native workers and one to two blocking workers.
  A real runtime-owned application child region owns native effects. Request
  polls install that region's context; spawned tasks receive their own context.
- Session, torrent runs and magnet metadata resolution have child regions.
  Pause and close cancel and drain them. Peer-slot permits are acquired and
  dropped in the peer task that owns them; one outgoing task may wait for
  capacity, preserving the 96-peer limit.
- Owned hashing and disk work run through bounded native blocking work.
  Initial verification checkpoints between pieces and observes physical work
  completion before admitting a new check. Files are hash checked on opening;
  startup cost therefore scales with the amount of existing data.
- The single-instance lock is acquired before reading records or starting
  transfers. A private Unix socket connects CLI/widget requests to the open app.
  At most 16 synchronous request workers are owned and joined by the server.
- Closing stops socket admissions, cancels pending engine requests, pauses and
  drains torrent/session work, joins request workers, closes the application
  region and stops the runtime. Drain deadline expiry is reported as failure.
- Torrent metadata and policy records use fsync plus atomic replacement and a
  directory sync. Adds save a paused record before admitting payload transfer.
  Deletion saves record removal before deleting payload, preventing resurrection
  after a record-write failure. Invalid/corrupt records are preserved and cause
  a clear startup failure.
- Saved hashes and metadata are validated before download files are opened.
  Unix file creation and deletion are anchored to the user-selected output
  directory; symlink descendants are refused. Legacy import stages paused
  records, checks metainfo/paths/selection, preserves the source, and retires
  qBittorrent only after the import is durably saved.

These are implementation bounds, rather than measured optimal settings.

## App controls

| Surface | Behavior |
| --- | --- |
| Magnets and torrent files/URLs | Native TCP/BEP9 metadata; bounded, cancellable lookup. Browser file URLs preserve literal plus signs, spaces and Unicode. |
| Discovery | DHT, HTTP/HTTPS and UDP trackers, local peer discovery and peer exchange. TCP incoming connections; no automatic router forwarding. |
| Pause/resume/remove | App routes with durable state. File deletion requires the normal CLI safety confirmation. |
| Default upload policy | 32 KiB/s payload upload while downloading; stop at completion. Explicit seeding removes the cap and permits continued activity. |
| Custom upload cap | Zero is unlimited. Positive caps require at least 16 KiB/s, the engine's protocol-chunk reservation. Changing a cap rebuilds/checks that torrent. |
| Scheduling and files | FIFO queue with three simultaneous downloads by default and live adjustable limit. Manual pause is durable and separate from queue suspension. Completed seeders do not consume slots. Sequential per file, first/last pieces prioritized; include/skip file selection. High/max priority and disabling sequential scheduling are unavailable. |
| Share limits | Unlimited or stop at completion. Other ratio thresholds are rejected. |
| Peer encryption / transport | Standard MSE for incoming/outgoing peers and BEP9 metadata. Prefer defaults to RC4 with a fresh-connection plaintext fallback; Require rejects plaintext and never downgrades. Off uses plaintext. uTP is unavailable. |
| VPN interface binding | Exact Linux kernel device binding for engine TCP/UDP, trackers, discovery, torrent HTTP/TLS and DNS. Proton follows `proton0`; missing/down/replaced interfaces stop and drain the session, then restore it only when ready. No ordinary-interface fallback. A waiting app serves saved history without a torrent session. |
| Upstream service APIs | Web server, UPnP, SOCKS, SQL persistence and upstream fast resume are outside this integration. |
| Metainfo | v1 and compatible v1 hashes; pure BitTorrent v2 migration is refused. |
| Swarm statistics | Connected peer count available; swarm-wide seed/leecher split unavailable. |
| Search | Local provider preferences; bounded Python jobs. Feed/search/title websites use usual networking. When engine VPN binding is enabled, external provider download hooks are refused; magnets and direct torrent URLs are added through the engine. All subprocess groups belong to the invoking app/CLI. |
| Shows and RSS | Local durable history, manual adds. Automatic checks occur on app opening, with explicit refresh available. |

## Final review and verification

The JSM Rust port guidance is applied to behavior, edge cases and the supported
surface above. Asupersync guidance supplies native ownership/cancellation tests.
The port verification skill supplies independent fresh review, fault checks
and explicit capability limits. The profiling/optimization skills require
measurements before performance changes. This is internal qualification of the
client's used surface; full upstream parity, long WAN soak and comparative
engine benchmarks remain open.

Reproducible checks use the repository's pinned Rust 1.98.0 toolchain.
The core crate defaults to the same `sha1-ring` backend as librqbit's
`rust-tls` feature, so selecting both packages in one invocation is supported.

```sh
# App behavior, actual magnet/payload transfer, persisted policy, source parsing,
# local feeds/shows/search, pointer/marquee, native context and parked file owners.
cargo test --locked -p fbittorrent -p rqbit-runtime -- --nocapture

# Native tracker/DHT/socket wire fixtures, timer cancellation and port release.
cargo test --locked -p librqbit-dualstack-sockets -p librqbit-tracker-comms \
  -p librqbit-dht --features librqbit-core/sha1-ring --lib

# Piece/hash boundary cases, malformed metainfo and real storage operations.
cargo test --locked -p librqbit-core -p librqbit --features librqbit/rust-tls --lib

# The same engine libraries can also be checked separately.
cargo test --locked -p librqbit-core --lib
cargo test --locked -p librqbit --features rust-tls --lib

# Real CLI/TUI/RSS actions and close/reopen in isolated temporary state.
python3 agent_ergonomics_audit/audit/regression_tests/native.py target/release/fbittorrent

# Optimized, symbol-preserving measurements, isolated data and normal discovery.
RUSTFLAGS='-C force-frame-pointers=yes' cargo build --profile release-perf
python3 agent_ergonomics_audit/audit/regression_tests/resources.py \
  target/release-perf/fbittorrent --output /tmp/fbittorrent-resources.json
```

The main regression script rejects background worker panics/obligation leaks
alongside a failing exit code; a green caller thread alone is insufficient.
The local magnet fixture transfers a generated 256 KiB payload, applies a cap,
interrupts the app, reopens its ledger and compares exact bytes. Other tests
cover stalled HTTP cancellation through an actual IPC worker, corrupt records,
metadata mismatch before file creation, failed deletion persistence, provider
child cleanup, symlink sentinels and seeded native Lab cancellation.

Final receipts and measured resource bounds are recorded in
`docs/validation/2026-10-09/README.md`, with queue, encryption and binding evidence
in `docs/validation/2026-10-10/README.md`. WAN/private-tracker interoperability,
large-library startup cost, heap allocation peaks and sustained throughput are
outside the local measurements.
