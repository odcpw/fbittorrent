# Local tracker diagnostic patch

S09 extends `librqbit-tracker-comms::stats::{redact_url,sanitize}` to remove URL
usernames, passwords and fragments as well as query strings. Known credential
values are removed if an untrusted tracker echoes them outside a URL. Existing
status storage, force coalescing and callers retain their behaviour. librqbit
reexports both helpers so API/CLI/TUI presentation uses the same sanitizer.

Tests: `cargo test --locked -p librqbit-tracker-comms` (including
`userinfo_and_fragments_never_reach_tracker_diagnostics`), fbittorrent's
`api::trackers` tests, and the private tracker journeys for both UIs. The stored
tracker URL remains complete internally; rendered URLs and
diagnostic text contain no userinfo, query string or fragment.


# Tracker path credential privacy (P6-01/P7-04)

`librqbit-tracker-comms::redaction` centralizes the existing public helpers and
redacts hex/base64 path segments of at least 16 characters, named credential path
parameters, percent-encoded forms and credential values echoed in remote failure
prose. Userinfo, queries and fragments remain private. Monitor spans, HTTP/UDP
errors, observations and Debug output all use this producer-side boundary before
logging; original announce URLs and tracker selection remain unchanged internally.

Test: `cargo test --locked -p librqbit-tracker-comms`;
`cargo test --locked -p librqbit --no-default-features --features rust-tls tracker_stats -- --test-threads=1`
exercises real HTTP/UDP failures, unmodified announce credentials and captured traces;
the application's `api::trackers` and engine alias-removal tests cover wire identities; `plugin/tests/tracker_privacy.py`
checks real API/TUI/dashboard/popup and diagnostic output with synthetic secrets.


# Remote-input hardening (security pass 2026-10-19)

- `librqbit-bencode`: `BencodeDeserializer` counts list/dictionary nesting and
  fails with `DepthLimitExceeded` above `MAX_DEPTH` (64). Ignored DHT
  arguments, tracker bodies, peer extension messages and fetched .torrent files
  all recursed once per level before, so a crafted datagram or body overflowed
  a 2 MiB worker stack and aborted the daemon.
- `librqbit` ut_metadata: a request for piece `total_pieces` (one past the end)
  is refused instead of panicking in `Bytes::slice`; metadata data received
  before an extended handshake or after completion is an error, not an unwrap.
- `librqbit` magnet metadata: a peer's announced `metadata_size` must be
  non-zero, and all peers share one 64 MiB in-flight allocation budget.
- `librqbit` storage: the torrent-named subfolder the session derives under its
  own folder is opened with `O_NOFOLLOW` through its parent, and only that
  derived folder is removed by delete-with-files. A user-selected output folder
  (including a single-file torrent's save path) is followed and never removed.

Tests: `cargo test --locked -p librqbit-bencode -p librqbit-dht -p librqbit-tracker-comms nest`;
`cargo test --locked -p librqbit --no-default-features --features rust-tls --lib -- metadata_piece derived_ budget_tests torrent_paths_cannot selected_folder_can`.

# Queued peer requests (engine performance, 2026-10-20)

The peer writer groups up to sixteen already-queued Request frames in its
existing buffer. It does not wait to fill a batch, reorder requests, cross
control/policy messages, or remove cancellation/interface-binding checks.
The focused `request_batch` tests decode wire fields/order and exercise control
barriers and empty/closed queues with the application's native-runtime features.

Repeated private 1 GiB measurements show fewer allocator calls and fewer normal
read/write calls with four/sixteen seeds; they do not establish a throughput or
CPU gain. This is an unqualified candidate: strict shutdown/workflow failures
remain red. See `tools/engine-bench/RESULTS.md` at the repository root for the
before/after measurements, behaviour proofs and retained failures.

# Verified container metadata demand (watch while downloading, 2026-10-20)

`ManagedTorrent::prioritize_metadata` supplies one validated file-relative byte
range to the existing streaming priority iterator, without opening payload files,
creating a reader or changing the saved file selection. The range is represented
by two piece indices, traversed lazily, and may be replaced or cleared. This lets
the daemon first fetch an MP4 tail box header and then every piece of its complete
moov, even when it spans many small pieces; first/last-piece priority alone missed
the header. Existing verified reads and normal file ordering are unchanged.

Tests: `cargo test --locked -p librqbit --no-default-features --features rust-tls
 torrent_state::streaming::tests -- --nocapture`; app `daemon::watch` tests;
`python3 agent_ergonomics_audit/audit/regression_tests/watch_player.py
 target/stabilize-debug/debug/fbittorrent --case mp4_tail` exercises real magnet
review, early readiness, full verified moov and actual mpv playback with 16 KiB
pieces. The same runner covers seek, pause, slow peer, EOF and shutdown.
