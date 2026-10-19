# fbittorrent API v1

This is the maintained contract for `$XDG_RUNTIME_DIR/fbittorrent/api.sock`.
`src/api/` contains P1 wire types. `contracts/ipc/v1/` supplies shared Rust,
daemon and UI replay examples. P1-05 defines types and fixtures; it starts no
listener. The existing legacy `engine.sock` protocol remains independent.

## Framing

One UTF-8 JSON object per line terminated by `\n`. Input is at most 65,536
bytes; server output at most 1,048,576 bytes. Malformed JSON, invalid UTF-8,
non-object input and overlong input receive a request-less invalid_message,
then bye and close. Snapshot chunks contain at most 256 items. The output cap
applies to every server frame. The plugin subscribes for collections.

No binary payloads: .torrent input is a path, opened O_NOFOLLOW, required to be a
regular file at most 16 MiB whose first byte is d (bencoded).

## Envelopes and resend policy

- Requests: `{"id": u64 ≥ 1, "op": str, "args": object?, "ref": str? (≤ 64 ASCII), "instance":
  str?}`. `args` may be omitted when an operation takes none. `instance` is the daemon instance
  the client last saw in `hello`; clients send it on every resend after a reconnect so the daemon can tell a request that predates its own start (P1-09).
- Responses: `{"id", "ok": true, "result": object}` or `{"id", "ok": false, "error": Error}`.
- Events: `{"ev": str, …}`; subscription events carry `sub`, `topic` and `rev`.
- An error with no request to answer (a line over the cap or not an object, `hello_required`,
  `busy` for the 33rd connection, `proto_unsupported`) is sent as the event
  `{"ev": "error", "error": Error}` and is always followed by `bye` and the close (A.8);
  `invalid_args` for a bad `id` (0, negative, non-integer, reused) answers with `"id": null`
  and keeps the connection.
- Times are Unix seconds (`*_at`); rates are bytes per second (`*_bps`); sizes are bytes; progress is
  a number from 0 to 1; durations are seconds (`*_s`).
- Full objects (snapshot items, `added`, results) never contain `null`: an unknown field is omitted.
  In a `delta`, a field set to `null` means "this field is now absent"; the client deletes it.
- Hashes are 40 lowercase hex characters (BitTorrent v1 info hash).
- Lines: client → server ≤ 65,536 bytes; server → client ≤ 1,048,576 bytes.
- Idempotent operations (safe to resend with the same `ref` after a mid-request loss, or
  against a new instance after a crash): `torrents.start`, `torrents.stop`,
  `torrents.set_seed`, `torrents.set_files`, `torrents.set_upload_limit`, `torrents.add`,
  `torrents.commit`, `torrents.discard`, `torrents.force_start`, `app.settings.set`,
  `transfer.set_limits`, `transfer.set_alt`, `search.set_provider`, `rss.set_read`,
  `rss.mark_feed_read`, `shows.set_quality`, `shows.set_read`, `shows.mark_show_read`,
  `torrents.reannounce`, `torrents.set_download_limit`, `torrents.add_tracker`,
  `torrents.remove_tracker`. Every other
  mutating operation (`torrents.remove`, `torrents.recheck`, `torrents.move`, `torrents.rename`,
  `torrents.queue_move`, `rss.add`, `rss.remove`, `shows.follow`, `shows.unfollow`,
  `shows.undo_read`, `search.install`, the `*.grab` family, `app.shutdown`, `app.restart`) is not,
  and a client never resends it on its own. `api::ops::TABLE` carries this flag and `api --list`
  prints it (P1-18); this list is the source.

`instance` is exactly 32 lowercase hex characters. Invalid/reused IDs get
invalid_args with id:null and details.key:id; the connection remains open.
Responses may arrive out of order. The failure envelope ID is the only nullable
output field outside a delta deletion patch. The complete idempotent list above
includes future operations. `api::ops::TABLE` currently covers only P1 operations;
each later addition must add a typed operation, metadata row, fixture and changelog.

## Handshake

First request: hello within 5 seconds. The server speaks exactly major 1.
Omitted interactive means false. Client kinds are cli, agent, tui, bar and test:
the bar connection also serves the dashboard, so there is no dashboard kind.
app.client updates interactive/glance until disconnect. The server reports
only implemented caps. Clients disable absent features and show a restart banner
when a required cap is missing.

| Direction | Fields |
|---|---|
| args | `proto: u64` (required; 1), `client: {name: str (≤ 64), version: str, kind: enum(cli\|agent\|tui\|bar\|test)}` (required;), `interactive: bool` (default false) |
| result | `proto: u64`, `server: {version: str, build: str, pid: u64, launcher: enum(systemd\|foreground), log: str}` (`launcher` is `systemd` when `NOTIFY_SOCKET` was set at start; `log` is `journalctl --user -u fbittorrent` under systemd and `the terminal running fbittorrent serve` otherwise, ready to print as a hint), `engine: str`, `instance: str` (32 lowercase hex characters from 16 bytes of `/dev/urandom`, read with `std::fs`; no uuid crate), `engine_epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `caps: [str]`, `limits: {line_in: u64, line_out: u64, inflight: u64, subs: u64, hashes: u64}`, `policy: {idle_exit_secs: u64}` (0 means the idle net is off) |

## Capability catalogue

| Cap | Operations and topics it covers | Since |
|---|---|---|
| `torrents` | `torrents.list/get/add/start/stop/remove/open_target`, topic `torrents`, `summary` | P1-06 |
| `files` | `torrents.files/set_files`, topic `torrent:<h>.files` | P1-06 |
| `subscribe` | `subscribe/unsubscribe`, snapshots and deltas | P1-08 |
| `mutations` | `ref` replay, `mutation.result`, pending states | P1-09 |
| `log` | Reserved for P1-14 | P1-14 |
| `settings` | `app.settings.get/set`, topic `settings` | P3-05 |
| `staging` | `stage: true`, `torrents.commit/discard` | P3-06 |
| `search` | `search.*`, topic `search:<id>` | P3-01 |
| `rss` | `rss.*`, topics `rss.feeds`, `rss.items:<feed>` | P3-02 |
| `shows` | `shows.*`, topics `shows`, `shows.releases:*` | P3-03 |
| `imdb` | `imdb.lookup` | P3-04 |
| `peers` | `torrents.peers`, topic `torrent:<h>.peers`, `seeds` field | P5-01 |
| `recheck` | `torrents.force_start` | `hashes: [hash]` (1..256), `on: bool` | `{state: "done"}` | yes | P6-05 |
| `torrents.queue_move` | `hash`, `to: enum(top\|up\|down\|bottom)` | `{state: "done", queue_pos: u64}` | yes | P6-05 |
| `torrents.recheck` | P5-03 |
| `limits` | `transfer.set_limits/set_alt`, `summary.alt`, `summary.limits` | P5-04 |
| `watch` | `playable` fields, `torrent.playable` event, `wait --until playable` | P5-05 |
| `trackers` | `torrents.trackers/reannounce`, topic `torrent:<h>.trackers` | P6-01 |
| `pieces` | `torrents.pieces`, topic `torrent:<h>.pieces` | P6-02 |
| `history` | `summary.history`; statistics reserved for later beads | P6-03, P6-04 |
| `queue` | `torrents.force_start/queue_move`, `queue_pos`, `force` | P6-05 |
| `origin` | `origin` field, `{restore: hash}` undo source | P6-08 |
| `clipboard` | `clipboard.magnet` event, `app.clipboard.dismiss` | P6-13 |
| `torznab` | provider kind `torznab` | P7-01 |
| `move`, `rename`, `tracker_edit`, `download_limit`, `deferred_add` | the P7 operations | P7-02..P7-09 |

## Versioning

Proto is the major; no minor exists. Caps negotiate features. Additive changes
include operations, topics, caps, optional output fields and new open-enum
values. A major cannot remove/rename fields, change types/units/meanings, or
make an optional input required. Input is strict: unknown op is unknown_op;
unknown/mistyped argument is invalid_args naming the key. All argument structs
use deny_unknown_fields. Output ignores unknown fields. Open reason/origin/log
source/busy-reason strings retain future values, rendered as “other” by clients.

## Errors

`Error = {code: str, message: str (≤ 200), hint: str?, retryable: bool, details: object?}`.

| Code | Meaning | Retryable | `details` | CLI exit |
|---|---|---|---|---|
| `invalid_message` | not JSON, not an object, or over the line cap | no | `{bytes?}` | 2 |
| `hello_required` | the first frame was not `hello`, or it came too late | no | | 2 |
| `proto_unsupported` | unsupported major version; the hint says what to update | no | `{server_proto}` | 2 |
| `unknown_op` | unknown operation; the hint lists up to three near names | no | `{nearest: [str]}` | 2 |
| `invalid_args` | missing, unknown or mistyped argument | no | `{key, expected?}` | 2 |
| `not_found` | unknown hash, feed, show, search session, file index or ref | no | `{hash?\|feed?\|imdb?\|search_id?\|index?}` | 6 |
| `ambiguous` | a selector matched several objects (raised by the CLI before calling) | no | `{matches: [str]}` | 7 |
| `safety` | destructive intent not confirmed (CLI `--yes` path) | no | | 3 |
| `invalid_state` | the operation does not apply now (resume on `done`, commit on a non-staged torrent, remove after a restart with a stale ref) | no | `{state?, reason?}` | 1 |
| `duplicate` | the torrent already exists (only as a `result` field for `add`; never an error) | n/a | | 0 |
| `unsupported` | refused by design (high or max file priority, sequential toggles) | no | | 2 |
| `engine_unavailable` | the engine is starting, parked or stopping | while starting: yes | `{state, message}` | 4 |
| `already_running` | a second `serve` while one holds the lock | no | `{pid?}` | 1 |
| `upstream` | a provider, feed, tracker or website failed | yes | `{source}` | 5 |
| `timeout` | the operation did not finish in time; its outcome is unknown | yes | `{after_s}` | 8 |
| `busy` | a limit was reached (connections, sessions, dispatch queue) | yes | `{limit}` | 4 (the hint says "retry in a moment"; 1 would contradict `retryable`) |
| `internal` | a bug; the hint points at `fbittorrent log` | no | | 1 |

Every CLI exit code 1..8 is reachable from this table.

Duplicate is never an error: add returns ok:true with duplicate:true. Retryable
is always present. Engine-unavailable is retryable only while starting. Upstream,
timeout and busy are retryable; a retryable error never maps to CLI exit 1.
Hints give a concrete next step when available.

## P1 operations

No arguments means args may be omitted or {}. Other rows use the named argument
object. Pending mutations require ref, generated by the server if the client
omitted it. Pending shutdown/restart have a required ref field; add/remove/upload
results use a tagged state whose pending variant requires ref. This shared rule
extends each pending result in the table below.
For upload-limit bps, omission leaves it unchanged and explicit null means
unlimited; a numeric limit must be at least 16384 bytes/second.

| Op | Args | Result | Mutating | Since |
|---|---|---|---|---|
| `hello` | See handshake | HelloResult | no | P1-06 |
| `app.info` | none | hello's `proto`, `server`, `engine`, `instance`, `engine_epoch`, `state`, `caps`, `limits`, `policy` plus `uptime_s: u64`, `upgrade_pending: bool`, `clients: {cli: u64, agent: u64, tui: u64, bar: u64, test: u64}` (connections since start), `subscriptions: {total: u64, by_topic: {str: u64}}` (live subscriptions, P1-08) | no | P1-06, P1-08 |
| `app.client` | `interactive: bool?`, `glance: bool?` (either or both; a connection drop resets both) | `{}` | no | P1-06 |
| `app.shutdown` | `reason: str?` | `{state: "pending"}`, then `bye` | yes | P1-06 |
| `app.restart` | `scope: enum(engine\|daemon)` | `{state: "pending"}` | yes | P1-06, P1-17 |
| `app.settings.get` | none | `Settings` | no | P3-05 |
| `app.settings.set` | any subset of writable `Settings` keys | `{applied: [str], restart: bool}` | yes | P3-05 |
| `app.network` | none | `{state: NetworkState, interfaces: [str]}` | no | P1-06 |
| `transfer.info` | none | `{dl_bps: bps?, ul_bps: bps?, session_downloaded: bytes, session_uploaded: bytes}` | no | P1-06 |
| `torrents.list` | `states: [state]?`, `origin: [str]?` | `{torrents: [Torrent]}` | no | P1-06 |
| `torrents.get` | `hash: hash` | `{torrent: TorrentDetail}` | no | P1-06 |
| `torrents.add` | `source: {magnet: str} \| {url: str} \| {path: str} \| {restore: hash}` (exactly one), `stage: bool?`, `stopped: bool?`, `seed: bool?`, `save_path: str?`, `files: [u64]?`, `category: str?`, `origin: open(manual\|search\|rss\|shows\|clipboard\|handler\|agent)?` | `{state: enum(pending\|done), hash: hash?, name: str?, duplicate: bool?, deferred: bool?}` | yes | P1-06, P1-09, P3-06, P7-09 |
| `torrents.recheck` | `hashes: [hash]` (1..256) | `{state: "pending", ref}`; then `mutation.result` with `{rechecked: [hash], failed: [{hash, message}], have_bytes: {hash: bytes}}` | yes | P5-03 |
| `torrents.start` | `hashes: [hash]` (1..256), `force: bool?` | `{state: "done", started: [hash], skipped: [{hash, reason: open(complete_no_seed\|already_running\|staged)}]}` | yes | P1-06, P6-05 |
| `torrents.stop` | `hashes: [hash]` | `{state: "done", stopped: [hash]}` | yes | P1-06 |
| `torrents.remove` | `hashes: [hash]`, `delete_files: bool` (required) | `{state: enum(done\|pending), undo_until: ts?}` (pending when deleting files; `undo_until` only when files are kept) | yes | P1-06, P1-09, P6-08 |
| `torrents.files` | `hash: hash` | `{files: [File]}` | no | P1-06 |
| `torrents.set_files` | `hash: hash`, `include: [u64]?`, `skip: [u64]?` | `{state: "done"}` | yes | P1-06 |
| `torrents.set_seed` | `hashes: [hash]`, `on: bool` | `{state: "done"}` | yes | P1-06 |
| `torrents.set_upload_limit` | `hashes: [hash]`, `bps: bps?` (≥ 16384, or `null` for unlimited) | `{state: enum(done\|pending)}` (pending until P6-07) | yes | P1-06 |
| `torrents.open_target` | `hash: hash`, `file: u64?` | `{path: str, kind: enum(file\|folder)}` | no | P1-06 |
| `subscribe` | `topics: [str]` (1..16), `interval_ms: u64?` (250..10000; applies to every topic of the call and is ignored by the "on change" topics of A.7; when omitted, each topic uses its A.7 default), `since_seq: u64?` (only with the `events` topic, A.7) | `{sub: u64, topics: [str]}` | no | P1-08 |
| `unsubscribe` | `sub: u64` | `{}` | no | P1-08 |

## IMDb (P3-04)

| Operation | Arguments | Result | Mutating | Since |
|---|---|---|---|---|
| `imdb.lookup` | exactly one of `name: str` (1..1024 characters) or `id: str` (`tt` + 1..10 digits), `refresh: bool?` | `{card: ImdbCard?}` | no | P3-04 |

`ImdbCard = {id: str, title: str, year: u16?, kind: str, rating: f64?,
votes: u32?, stars: str?, blurb: str?, wiki_url: str?, url: str,
poster_url: str?, poster_path: str?}`. Unknown ratings and unavailable posters
are omitted. `poster_path` names a local atomic cache file; clients load it as
`file://` without fetching network images. The index loads once per daemon;
`refresh: true` redownloads and replaces it before the response. Suggestion and
Wikipedia reads are capped at 2 MiB; posters at 8 MiB. Poster failure retains
the card without `poster_path` and logs a typed `upstream` diagnostic. Other
upstream failures return retryable `upstream` with `details.source`.

## P1 topics

| Topic | Kind | Key | Item type | Default interval | Used by |
|---|---|---|---|---|---|
| `summary` | state | — | `Summary` | 5,000 ms idle bar; 1,000 ms open | bar, popup, dashboard, TUI |
| `torrents` | keyed | hash | `Torrent` | 1,000 ms | popup (open), dashboard, TUI |
| `torrent:<h>.files` | keyed | `index` | `File` | 1,000 ms | details |
| `network` | state | — | `NetworkState` | on change | every UI |
| `settings` | state | — | `Settings` | on change | dashboard, TUI |
| `events` | keyed, append-only | `seq` | the unsolicited events of A.8 (`torrent.*`, `engine.changed`, `mutation.result`, `search.done`, `shows.checked`, `clipboard.magnet`) wrapped as `{seq, ts, ev, …}` | on change | `events --follow`, agents |

Subscribe accepts 1..16 topics. The supplied interval_ms is preserved on the
wire, and its effective sampling interval is normalized to 250..10000. The shared
`SubscribeArgs::effective_interval_ms(default)` performs that clamp; omitted
intervals use the table defaults and on-change topics ignore it. Unknown topic:
invalid_args with details.key:topics. Unknown detail hash: not_found. The result returns both sub and accepted topics.

Keyed snapshots are begin/chunk/end (at most 256 items/chunk). All chunks come
from one atomic state. No delta/state for that subscription may interleave with
the snapshot. The next delta carries intervening changes. Torrent snapshots sort
by added_at descending; files sort by index. Detail subscriptions exist only
while visible and their final unsubscribe releases sampling.

Rev is per subscription, shared across topics, and increases by exactly one for
every begin/chunk/end/delta/state. A gap triggers unsubscribe, subscribe and
rebuild from a fresh snapshot. Changed field:null deletes that field. Added and
snapshot items and whole state objects omit unknown fields.

Structural changes are emitted immediately on engine/ledger notifications.
Only progress, completed_bytes, uploaded_bytes, dl_bps, ul_bps, eta_s, peers,
seeds, ratio, playable.buffered_s, playable.finishes_in_s, all Summary rates/counts
and Peer/Tracker counters are coalesced to the interval. Everything else is
structural, including state, reason, name, ready and stall_free changes.

Outgoing queue: 512 frames. If full, attempt bye:slow_client once within 1 second
then close; never silently skip. After 15 seconds without a pushed frame, send
heartbeat. Clients reconnect after 45 seconds of silence. Subscribed sockets
have no idle-read deadline. Engine restart pushes engine.changed and then fresh
snapshots with reason:engine_restart.

The events ring retains 1000 entries or 10 minutes, whichever is smaller.
since_seq applies only to events, replaying from that sequence. A sequence older
than retention produces snapshot.begin with reason:resync and oldest_seq;
subscribe's reply is unchanged. Store (instance,seq); seq starts at 1 per instance.

## Events

| Event | Fields |
|---|---|
| `snapshot.begin` | `sub: u64`, `topic: str`, `rev: u64`, `count: u64`, `reason: enum(subscribe\|engine_restart\|resync)`, `oldest_seq: u64?` (only with `reason: resync` on the `events` topic, A.7) |
| `snapshot.chunk` | `sub`, `topic`, `rev`, `items: [object]` (≤ 256) |
| `snapshot.end` | `sub`, `topic`, `rev` |
| `delta` | `sub`, `topic`, `rev`, `changed: {key: {field: value \| null}}`, `added: [object]`, `removed: [key]` (each may be absent when empty) |
| `state` | `sub`, `topic`, `rev`, `data: object` |
| `engine.changed` | `epoch: u64`, `state: enum(starting\|running\|parked\|stopping)`, `reason: open(start\|settings\|network\|restart\|upgrade)` |
| `torrent.added` | `hash`, `name`, `origin` |
| `torrent.completed` | `hash`, `name`, `open_target: str` |
| `torrent.playable` | `hash`, `name`, `file: u64`, `open_target: str`, `buffered_s: u64?` |
| `torrent.error` | `hash`, `name`, `message: str` |
| `mutation.result` | `ref: str`, `op: str`, `outcome: enum(done\|failed\|timeout)`, `error: Error?`, `result: object?` |
| `search.done` | `search_id: u64`, `total: u64`, `warnings: [{provider: str, message: str}]` |
| `shows.checked` | `imdb: str?`, `new: u64`, `warnings: [{imdb, message}]` |
| `clipboard.magnet` | `hash`, `name: str?`, `already_added: bool` |
| `heartbeat` | `ts: ts` |
| `bye` | `reason: enum(shutdown\|idle\|restart\|upgrade\|slow_client\|hello_required\|invalid_message)` |
| `error` | `error: Error` (A.3); only for errors that have no request to answer (A.1); always followed by `bye` |

Unsolicited events (`torrent.*`, `engine.changed`, `mutation.result`, `search.done`,
`shows.checked`, `clipboard.magnet`, `heartbeat`, `bye`) are sent to every connection that has
completed `hello`, whether or not it subscribed to a topic; they are not coalesced and do not carry
`rev`. Every unsolicited event except `heartbeat` and `bye` carries `seq: u64` (daemon-global,
monotonic per instance) and `ts`, and is also appended to the `events` topic (A.7) so a client can
replay what it missed with `since_seq`.
Request-less error is a connection protocol frame, has no seq/ts and is not
replayed. Subscription frames use rev and have no daemon-global seq/ts.

## Mutation policy

Mutating operations accept ref. The daemon retains 256 refs per client.name for
10 minutes per instance, keyed by (client.name,ref). Repeating a ref returns the
stored response and never reapplies the operation. Distinct client names do not
collide. CLI/TUI process names include their PID; the single plugin uses the
stable name oliver.torrents so it can resend after reconnecting. A resend carries
the previously observed instance. Clients resend only idempotent mutations on
their own; destructive non-idempotent requests need fresh confirmation after an
instance change.

Synchronous mutations answer done after durable intent and engine application.
Long mutations answer pending with ref and any known hash, then mutation.result
with outcome done, failed or timeout. A missing client ref is generated by the
server and returned. Timeout means unknown outcome. UIs show progress until a
delta confirms the resulting state and never retry removal automatically.

Remove requires an explicit delete_files boolean. Hashes are explicit lists of
at most 256, never all. An add yields the hash immediately for magnet/path
sources; URL sources can remain pending while downloading. A duplicate add
succeeds with duplicate:true and the existing hash, changes no settings and
creates no second ledger record.

## Objects

| Object | Fields (? means omitted when unknown/not applicable) |
|---|---|
| `Torrent` | `hash`, `name: str (≤ 1,024 in lists)`, `state: enum(metadata\|checking\|downloading\|stalled\|queued\|paused\|done\|seeding\|waiting_vpn\|error)`, `reason: open(user\|queue_full\|complete_no_seed\|vpn_down\|restore_failed\|metadata_timeout\|missing_files\|disk_full)?`, `error: str?`, `intent: enum(run\|pause)`, `staged: bool`, `checked: bool`, `progress: f64?`, `size: bytes?`, `completed_bytes: bytes?`, `uploaded_bytes: bytes?` (this torrent's upload in the current engine life, from `stats.live`), `dl_bps?`, `ul_bps?`, `eta_s: u64?`, `peers: u64?`, `seeds: u64?`, `seed: bool`, `upload_policy: enum(default_cap\|unlimited\|custom)`, `upload_limit_bps?`, `ratio: f64?`, `save_path: str`, `content_path: str?`, `added_at: ts`, `completed_at: ts?`, `origin: open(…)?`, `imdb_id: str?`, `queue_pos: u64?`, `force: bool`, `playable: Playable?` |
| `TorrentDetail` | every `Torrent` field plus `file_count: u64?`, `piece_count: u64?`, `piece_size: bytes?`, `comment: str?`, `created_by: str?`, `private: bool`, `tracker_count: u64?`, `undo_until: ts?` |
| `Playable` | `file: u64`, `ready: bool` (the DL-26 threshold), `stall_free: bool?` (at the current `dl_bps`, the rest of the file arrives before playback reaches the end of the prefix: `prefix_bytes / bitrate ≥ (file_size − prefix_bytes) / dl_bps`; absent when no duration is known or the torrent is not downloading), `prefix_bytes: bytes`, `buffered_s: u64?` (`prefix_bytes / bitrate`, with `bitrate = file_size / duration`), `finishes_in_s: u64?` (`(file_size − prefix_bytes) / dl_bps`, the right-hand side of the rule; absent when `dl_bps` is absent or 0), `acked: bool` |
| `File` | `index: u64`, `path: str`, `size: bytes`, `progress: f64`, `included: bool`, `sort_key: str` (computed by the daemon in `torrents.files` and the files topic, P1-06: the path lowercased with every run of ASCII digits left-padded with zeros to 10 digits, so that a plain string comparison yields natural order: `e2.mkv` < `e10.mkv`), `ready: bool?` (P5-05) |
| `Summary` | `dl_bps?`, `ul_bps?`, `counts: {total, downloading, stalled, metadata, checking, queued, paused, done, seeding, waiting_vpn, error, staged, ready}` (all `u64`), `alt: bool`, `limits: Limits`, `network: {state: enum(ready\|waiting\|off), message: str, interface: str?}`, `daemon: {state: enum(starting\|running\|parked\|stopping), idle_exit_in_s: u64?, busy_reasons: [open(downloading\|seeding\|parked_waiting\|interactive\|search\|mutation)], upgrade_pending: bool}`, `top: [{hash, name, state, progress?, dl_bps?, eta_s?, playable?}]` (≤ 5), `disk: {free_bytes: bytes?, needed_bytes: bytes, low: bool}` (P5-07; the one place free space lives), `history: [u64]?` (last 60 download samples, P6-03) |
| `Limits` | `download_bps?`, `upload_bps?`, `alt_download_bps?`, `alt_upload_bps?` |
| `NetworkState` | `selection: str?`, `interface: str?`, `ready: bool`, `message: str` |
| `Settings` | `max_active_downloads: u64`, `peer_encryption: enum(off\|prefer\|require)`, `network_interface: str?`, `save_path: str`, `seed_by_default: bool`, `autostart: bool`, `idle_exit_secs: u64` (0 disables the idle net), `notify: bool`, `timeout_secs: u64`, `handler: enum(add\|confirm)`, `open_command: str` (default `xdg-open`), `clipboard_watch: bool`, `reduce_motion?: bool` (absent = false), `peer_port: u64`, `show_quality: str`, `download_limit_bps?`, `upload_limit_bps?`, `alt_download_limit_bps?`, `alt_upload_limit_bps?`, `alt_limits_on: bool` (the persisted turtle switch; `transfer.set_alt` writes it), `restart_class: [str]` (the keys whose change restarts the engine). Every key is also the `config.toml` key: one vocabulary for the file, the API and `fbittorrent settings` |
| `LogEntry` | `seq: u64`, `ts: ts`, `level: enum(error\|warn\|info\|debug)`, `source: open(daemon\|engine\|queue\|api\|search\|rss\|shows\|imdb\|notify\|network\|watch\|clipboard)`, `hash?`, `message: str` |

## Unknown values

| Field | Absent when |
|---|---|
| `progress` | parked; metadata unknown |
| `size`, `files` | before metadata |
| `eta_s` | not downloading, or rate is zero |
| `seeds` | until P5-01 can count them; always present afterwards for live torrents |
| `peers`, `dl_bps`, `ul_bps` | engine not live for that torrent (parked, starting, dormant) |
| `ratio` | nothing downloaded yet |
| `playable` | not a video, metadata unknown, or the `watch` cap is absent |
| `free_space` | `statvfs` failed or returned a negative value |
| `rating` (IMDb), `uploaded_at` (search), `size` (RSS item) | the source did not provide it (today RSS shows "0 B" [RC #4]) |

Rust uses `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none")]`. A contract test
walks every fixture object and fails on `-1`, `8640000` or a zero in a field documented as
"absent when unknown". Rates are the one place where 0 is a true value (live and idle).
The sentinel scan reports an exact JSON pointer. It distinguishes observed zero
from unknown: live idle rates and live peer counts, indices/counts, file progress,
progress with known metadata and ratio with downloaded bytes can be zero.
Unknown values are omitted. Missing ETA is never 0 or 8640000. Unknown size,
rating, source timestamp and free bytes are never 0. Full objects never use null.

## Limits

| Limit | Value | Why |
|---|---|---|
| Connections | 32 | CLI bursts plus the plugin, a TUI and agents; today 16 one-shot workers (`src/ipc.rs:27`) |
| In-flight requests per connection | 8 | A slow `add` must not block a `stop` on the same socket |
| Dispatch pool | 8 threads, queue of 64; beyond that `busy` (retryable) | Bounded like today's server |
| Subscriptions per connection | 16 | The plugin's one connection carries the bar, the popup and the dashboard: `summary`, `torrents`, `network`, `settings`, `rss.feeds` and `shows` (badges) plus a view topic and up to two detail topics, nine on a bad day; the TUI mirrors the same six plus details (P4-03). 8 would refuse the ninth with `busy`; 16 costs nothing and is still a bound |
| Outgoing queue per connection | 512 frames | |
| Concurrent search sessions | 4 | Each is one nova2 process plus a `spawn` pool of `min(cpu_count, 4)` workers that run the enabled providers (`vendor/nova3/nova2.py:71-75,262-264`): at most about 5 processes and 150 MB per session |
| Hello deadline | 5 s | Drops clients that never speak |
| Default op timeout | 15 s; `torrents.add` 95 s | Matches today's add timeout (`src/ipc.rs:96-100`) |
| Ref ring | 256 per client name, 10 min | |
| Log ring | 2,000 entries | |
Client deadlines are 20 seconds normally, 100 seconds for add; daemon operation
timeouts are shorter so clients receive a typed timeout. Hash lists contain
1..256 hashes and never all.

## Fixtures and tested examples

Each line is c2s/frame (Request), s2c/frame (typed reply/event), expect/model
(expected client store) or a first-line volatile/paths. Paths are relative to a
frame (for example result.instance). Live replay uses the declared normalization below; typed round
trips mask nothing. Files are independent conversations. A repeated hello in
hello.ndjson begins a new mismatch-banner scenario for proto 2, not a serve flag.

Expected models have optional proto/compatible, torrents keyed by hash, summary,
network, settings and subscriptions keyed by decimal sub ID. A subscription
expectation has topic/rev/resync. subscribe-gap deliberately injects a skipped
revision: the suspect delta is not applied, resync becomes true, then c2s
unsubscribe/subscribe and a new snapshot rebuild the store. This is a reducer
fixture, not a legal server transcript. errors injects each representative
failure; its invalid-ID and request-less failures are input-validation scenarios.


**Live replay (P1-06, orchestrator ruling 3).** The hello/errors/torrents runner
generates real payloads and metainfo from transcript names, sizes, files and
selection, adds them through the API, and drives the available state/error
preconditions. The test owns its literal /tmp/fbt-fixture paths and an isolated
user/network namespace; it never adopts an existing directory or edits a ledger.
The following single table is defined in daemon.py and reproduced verbatim here
and in the P1-06 receipt. Identity bindings are bijective and checked at every
occurrence. Volatile fields retain their presence/type/validity checks. All other
fields asserted by the fixture compare byte-for-byte after canonical P1-05
typed re-serialization. Under ruling 4C an optional member absent in the
fixture may be present live: the frozen Rust checker validates the live frame
and its form with that member removed. Every occurrence is logged as "extra
optional field" with its JSON pointer and reproduced verbatim in the receipt.
Missing or changed fixture-asserted members still fail. A
future-capability/proto-2/unreachable-error frame is validated with the frozen
P1-05 checker and recorded by file, line, JSON pointer and target bead. No other
mismatch is deferred. A failed precondition or non-deferred mismatch fails replay.
Ruling 6 accepts the ruling-5 server behavior. P1-05b/P1-05c's fixture corrections
are integrated: exact payload/file sizes feed the generator, and the opted-in
network replay explicitly binds fixture0, yielding selection/interface `fixture0`,
ready true and message `VPN bound: fixture0` inside the owned VPN namespace
(`vpn.py`, `FBT_TEST_VPN=1`). Without that opt-in, the environment-conditional
deferral remains accepted. Recheck/query attempts are bounded and logged;
downloading uses a real loopback native seed and tracker. When receiving geometry
is unspecified and the fixture asserts zero verified bytes, 1 MiB pieces allow a
positive measured chunk rate before the first piece verifies. Missing-file restore
preconditions complete the payload, delete it, restart dormant history and
explicitly activate it, preserving startup's lazy restore policy. The diagnostic
remains `<name> restore failed: missing files`, with reason `missing_files`.
Ruling 7 treats checking measurements as state-conditioned volatile values:
when the live torrent is checking, completed_bytes must be a u64 no greater than
size, and finite progress must equal completed_bytes/size within 1e-9 absolute
(zero size requires both zero). Presence is preserved. Checked and size remain
exact; other states still compare completed_bytes/progress exactly. P1-05d's
fixture correction to checked=false for an active check is integrated.
Live validation checks every original frame against the frozen types, bounds,
capabilities and canonical round trips, and retains revision checks. Its
cross-reply agreement still checks static sizes and file totals. For any hash
observed checking, its measurements are excluded from that history's agreement,
including earlier samples and later cancellation; their raw values remain
validated and non-checking fixture comparisons remain exact. Fixture transcripts
continue to use the unchanged strict whole-group agreement hook. Deferrals record
file:line, pointer, class, target bead and reason. Capability/protocol deferrals
apply only while that feature is unavailable; advertised features stay under
active comparison. The receipt distinguishes undrivable catalogue frames from
error codes that can be produced with a different shape and retains the bounded
starting-window retry evidence. A retained missing-files restore error keeps its
missing_files reason through Stop and retry until the error clears. Nonvolatile
validation failures are recorded without a new mask or deferral.

| JSON pointer(s) | Comparison | Validity |
|---|---|---|
| `/id` | identity:request | positive u64, consistent first-occurrence bijection |
| `/result/instance` | identity:instance | 32 lowercase hex, consistent first-occurrence bijection |
| `/**/hash; /result/started/*; /result/stopped/*` | identity:hash | 40 lowercase hex; API-generated hashes bound to fixture identities |
| `/ref; /result/ref` | identity:ref | at most 64 ASCII characters, consistent first-occurrence bijection |
| `/ts; /result/torrent/added_at; /result/torrent/completed_at; /result/torrents/*/added_at; /result/torrents/*/completed_at` | timestamp | positive u64; presence preserved |
| `/result/dl_bps; /result/ul_bps; /result/torrent/dl_bps; /result/torrent/ul_bps; /result/torrents/*/dl_bps; /result/torrents/*/ul_bps` | u64 | nonnegative u64; presence preserved |
| `/result/torrent/eta_s; /result/torrents/*/eta_s; /result/torrent/peers; /result/torrent/seeds; /result/torrents/*/peers; /result/torrents/*/seeds` | u64 | nonnegative u64; presence preserved |
| `/result/session_downloaded; /result/session_uploaded; /result/uptime_s; /result/clients/*` | u64 | nonnegative u64; presence preserved |
| `/result/server/pid` | timestamp | positive u64; presence preserved |
| `/result/server/build` | text | nonempty string; presence preserved (declared fixture volatile path) |
| `/result/server/version` | version | semantic version string; presence preserved |
| `/result/torrent/completed_bytes; /result/torrent/progress; /result/torrents/*/completed_bytes; /result/torrents/*/progress` | checking only | live state=checking: u64 completed_bytes <= size, finite progress=completed_bytes/size within 1e-9 absolute; size=0 requires both zero; presence preserved; other states compare exactly. Checking hashes exclude these measurements from live-history agreement, retaining size/file totals |
| `object member absent in fixture` | extra optional field | live value validates against P1-05; removing the member also validates; log every pointer (ruling 4C) |
| `all other JSON pointers` | exact | canonical bytes through the P1-05 types; no extra masks or value substitution |

```ndjson
{"dir":"c2s","frame":{"args":{"client":{"kind":"test","name":"fixture-test","version":"0.2.0"},"interactive":false,"proto":1},"id":1,"op":"hello"}}
{"dir":"s2c","frame":{"id":1,"ok":true,"result":{"caps":["torrents","files","subscribe","mutations"],"engine":"librqbit 9.0.1","engine_epoch":1,"instance":"0123456789abcdef0123456789abcdef","limits":{"hashes":256,"inflight":8,"line_in":65536,"line_out":1048576,"subs":16},"policy":{"idle_exit_secs":300},"proto":1,"server":{"build":"fixture","launcher":"foreground","log":"the terminal running fbittorrent serve","pid":1234,"version":"0.2.0"},"state":"running"}}}
{"dir":"s2c","frame":{"ev":"bye","reason":"invalid_message"}}
```

`cargo test --locked api::fixtures` loads all seven files. roundtrip deserializes
and reserializes every typed line and checks successful results using their
request operation. no_sentinels enforces omission/sentinel rules. Coverage checks
pin the bead's 20 operations literally, require every A.8 event and all 16 error
codes, and enumerate present/absent examples for the optional object fields and
all ten torrent states. Documentation examples are parsed and compared to fixtures. Negative controls cover malformed
records, strict input, pointer errors and explicit null versus omission.
This is contract proof. P1-06 adds real daemon replay; P2-02 adds Store reducer
replay. A green typed fixture test alone is not live daemon or UI acceptance.


P4-06 adds the optional boolean `Settings.reduce_motion` to the `settings` state
object. Omission means `false`; `true` disables hover-name scrolling in the TUI,
and `false` enables its single pass. A present value must be a JSON boolean
(`null`, numbers and strings are invalid). This is the same `reduce_motion` key
as `config.toml` and the TUI Settings toggle. It is a UI preference and never
restarts the engine. No new capability is introduced: the field belongs to the
existing `settings` capability, advertised when that service is implemented.
Old clients can ignore it, and old daemons can omit it. Whole-state replacement
with the field absent resets the preference to `false`, including after a true
value. `settings-motion.ndjson` covers absent, true, false and absent again.

```ndjson
{"dir":"s2c","frame":{"data":{"alt_limits_on":false,"autostart":false,"clipboard_watch":false,"handler":"confirm","idle_exit_secs":300,"max_active_downloads":3,"notify":true,"open_command":"xdg-open","peer_encryption":"prefer","peer_port":6881,"reduce_motion":true,"restart_class":["network_interface","peer_port"],"save_path":"/tmp/fbt-fixture/downloads","seed_by_default":false,"show_quality":"1080p","timeout_secs":20},"ev":"state","rev":2,"sub":1,"topic":"settings"}}
```

## Value validation

The shared `api::validation` module checks topic/hash list counts, upload limits,
normalized intervals, since_seq topic eligibility, progress in 0..=1, list names
of at most 1024 characters, Error.message of at most 200 characters, Summary.top
of at most five entries, frame byte caps, and contiguous per-subscription
revisions. Request deserialization and contextual reply decoding invoke it;
fixtures also validate events, expected models and revision order. Pending
correlation refs are required by serde variants. The deliberate subscribe-gap
scenario requires one exact revision failure and recovery; it does not bypass
other validation. LogEntry examples in diagnostic error details exercise the
specified object without introducing its later operation or topic.

## Changelog

- **P3-04:** implements the planned `imdb` capability and `imdb.lookup`, adds
  `ImdbCard` with optional local `poster_path`, and an additive IMDb transcript.


- **v1 (P1-05):** establishes P1 envelopes, strict args, contextual result types,
  error/CLI-exit mapping, operation metadata, topic payloads, A.8 events and
  Torrent/TorrentDetail/File/Summary/Limits/NetworkState/Settings/LogEntry.
  Adds six fixture groups, proto-2 mismatch, request-less errors, pending refs,
  revision-gap recovery and executable documentation examples. Capability
  ownership is recorded above; each cap is advertised only when implemented.

- **v1 review correction (P1-05):** limits the catalogue to the bead's literal
  operation list, restores all hello fields on app.info, requires pending refs,
  completes optional-field/state/delta coverage, and adds reusable bound/revision
  validation with negative controls through the fixture decoder. The frame cap
  stays 1,048,576 bytes; P1-06 uses chunked collections.

### P6-07 — live per-torrent upload limits

The typed engine's existing `SetUploadLimit` operation now applies a validated
cap synchronously after its durable ledger save. It retains the torrent handle,
checked data and peers, and returns `EngineResult::Acknowledged` only after the
live limiter has been updated. Legacy `torrents/setUploadLimit` still replies
`Ok.`. Existing CLI `seed` / `noseed` use this path and no longer rebuild a
managed torrent to change its cap. Nonzero caps below 16384 bytes/s or above
the native `u32` maximum are rejected. Zero remains legacy unlimited.

For the planned v1 adapter, `torrents.set_upload_limit {hashes, bps?}` can return
`{state: "done"}` synchronously; keep the result type `enum(done|pending)` for
wire compatibility. `torrents.set_seed` must use the same live setter for
existing handles. The v1 adapter is not present in this checkout, so this entry
does not advertise a new operation, capability or active NDJSON endpoint.

### P7-09 — deferred adds while waiting for the VPN

The `deferred_add` capability enables parked `torrents.add` for magnets and local
`.torrent` paths. It saves the record and local metadata privately, keeps the
requested run/pause and upload policies, appends to queue order, and returns
`state:done`, the hash, and `deferred:true`. Rows remain `waiting_vpn` with unknown
progress until the interface returns. Activation resolves metadata and restores
the saved queue intent; the deferred marker clears after admission. Duplicate
adds preserve the existing record. URL sources are refused while parked before
fetching or returning a pending receipt. Characterization: `deferred-add.ndjson`;
live cases: `deferred_add_saved_record` and opt-in `deferred_add`.

### P4-06 — optional reduced motion

Adds `Settings.reduce_motion?: bool` (absent = false), preserving all earlier
fixture records and documentation examples. The new `settings-motion.ndjson`
conversation covers both presence and omission. Strict boolean decoding rejects
null and non-boolean values; explicit false remains present on a typed roundtrip.
This extends the v1 object contract without adding an operation or advertising
a not-yet-implemented service. The standalone TUI persists and applies the same
preference from `config.toml`; API-client attachment consumes it in P4-03.


### P1-11 — CLI client and contract 1.2.0

The CLI uses the bounded blocking API session, matches up to eight requests by
ID, keeps events separate, and decodes subscription revisions using the frozen
per-subscription counter across topics. Reads never open a daemon; offline stop
and keep-files removal share engine.lock and the durable parked ledger. A lost
idempotent request can reconnect once with the original ref and observed
instance; destructive unknown outcomes return timeout without replay. Native
add uses the returned hash and accepts `--origin manual|handler|agent`.
Contract 1.2.0 declares the stopped-status `ok:true` change and additive normalized
torrent fields while preserving the legacy numeric sentinels. Capabilities are
generated from clap, with starts/since metadata. JSON warnings identify legacy
mode and version skew. `SOURCE_DATE_EPOCH` removes CLI-derived elapsed time;
ledger timestamps are preserved. Live summary/idle/upgrade qualification follows
the P1-08/P1-12/P1-17 integration; cumulative app.info client counters never stand
in for live interactive presence.

P7-09 CLI completion: contract **1.2.1** advertises `deferred_add` in capabilities
and records the additive `add.deferred` field. `add --json` preserves the atomic
API reply's `deferred:true`; human output says “waiting for VPN”. The desktop
handler passes `--origin handler`. The API/engine implementation and new fixture
remain unchanged; the CLI uses the returned hash without list differencing.

### P3-05: durable settings operations

`app.settings.get` returns `Settings`. `app.settings.set` accepts any subset of
its writable keys, validates the whole patch with `Config::validate`, and names
an invalid key in `error.details.key` before making changes. `restart_class` is
read-only and equals `peer_encryption`, `network_interface`, `peer_port`.
An explicit null clears `network_interface` or an optional speed limit; other
keys require their declared type. Speed limits range from 16384 through
4294967295 bytes/second; absence means unlimited.

Settings changes are not pending mutations: no `mutation.result` follows them.
A restart-class change answers `{applied: [keys], restart: true}` after the durable
save, the supervisor restarts the engine slot, and clients observe completion
through `engine.changed` (and the `network` state when the binding changed).
Live changes answer `restart:false` after save and apply. Repeating an unchanged
patch returns an empty `applied` list and does not restart. The `settings` state
is published on change by P1-08; this branch exposes the settings generation and
snapshot for that integration. The CLI uses these operations when running and
saves the private config without starting the daemon when stopped.

Adds capability `settings`, the two operation-table rows, and the independent
`settings.ndjson` transcript. CLI contract 1.2.2 adds canonical settings values,
`applied`, `restart` and `running`; existing CLI settings keys remain available.

### P5-03: asynchronous verification

Capability `recheck` adds `torrents.recheck`. Admission refuses staged or
metadata-less torrents with `invalid_state`, unknown hashes with `not_found`,
and parked engines with `engine_unavailable`. It immediately acknowledges the
ref as pending; one terminal `mutation.result` carries measured verified bytes,
successes and per-hash failures. Work is engine-owned, serial by hash and outside
the dispatch pool/ledger lock during hashing, and has no operation timeout.
Concurrent requests for a hash already in flight share its check and outcome.
Checks preserve durable paused/run intent and take no queue slot. Completion is
cleared durably before payload I/O and remains absent when data is missing, so a
restart during repair cannot restore a corrupt record as dormant done. Restoring
a dormant record performs the check once. Engine restart/shutdown cancels and
joins this work before releasing its owner; clients receive the terminal receipt
before shutdown bye. The independent `recheck.ndjson` transcript and contract
patch 1.2.3 document the new operation without modifying earlier transcripts.
Dashboard/TUI/CLI surfaces are integrated by their owner tracks; completion
notifications and the pending busy reason consume P1-08/P1-12/P1-13's channels.

### P6-05: force start and queue order

Capability `queue` adds idempotent `torrents.force_start` and non-idempotent
`torrents.queue_move`, and implements `torrents.start {force:true}` (already-running
hashes remain in `started`). Force-on clears user pause and bypasses the ordinary
queue limit; force-off rejoins the ordinary queue. Stop and completion clear force.
Staged torrents are refused; completed stop-on-complete records give the explicit
`fbittorrent seed <hash>` hint. Failed records retry through the existing start
path. Parked force/order changes persist without opening a session.

Ledger order remains queue order. `queue_pos` is one-based among unforced,
unstaged records with running intent and no positive completion timestamp,
including metadata/error/waiting rows. Unknown negative legacy completion dates
remain incomplete. Up/down skip paused, completed and forced rows; top/bottom
move within positioned rows. A reorder protects transfers already occupying
ordinary slots until pause, stop, completion, error or an explicit limit decrease
releases them, while ordinary FIFO resume behavior is preserved. The independent
`queue.ndjson` transcript and contract patch 1.2.4 add these operations and row
fields without changing previous transcripts. Dashboard/TUI controls and CLI
verbs are separate surface integration work.
