# Track D — TUI

## P1-09 — fbt-p1-09-wsl

Implemented per-instance/per-client 256-entry, ten-minute ref receipts, exact
op/args matching, cached errors/pending responses, replay listeners, generated
refs and stale-instance checks from `ops::TABLE`. URL metainfo is validated
(bencoded, ≤16 MiB) before admission through the existing bound/cancellable
engine runtime. Magnets and file deletion have correlated pending outcomes;
file deletion has no timeout. The ring survives engine-slot restarts.
`mutations` is advertised; frozen contracts/docs are unchanged. Split the
inherited server into modules, all below 900 lines; framing/decoder bodies are
unchanged. Neighbouring `src/engine/ops.rs` adds only a per-hash try-lock restore
receipt and cancellable URL fetch; teardown is unchanged. The server split
will overlap P1-06/cod_2 and P1-08/cod_1's concurrent server changes.

Evidence root: `target/test-logs/fbt-p1-09-wsl/`. Commands use `run.py` there to
retain command JSONL/stdout and private XDG; daemons additionally isolate HOME
and TMPDIR and use local HTTP/payloads.

| Command | Result and retained evidence |
|---|---|
| `cargo check --locked` | Exit 0, `check.log` (before split). |
| `cargo test --locked -p fbittorrent api:: -- --nocapture` | Exit 0, 62 passed/1 ignored, `api_unit_final.{jsonl,log}`; expiry prints 599+1 seconds; includes eviction, cached errors, names and pending order. |
| `FBT_MUTATION_ARCHIVE=live_final python3 …/live.py` | Exit 0, 16 real-process cases/380 steps/zero failures; `live_final.{jsonl,log}`, `live_final/daemon/`. All seven implemented acceptance criteria pass in their named cases; CLI list-diff removal remains P1-11. Includes 257/empty hashes, base32/v2, generated refs, URL size/bencoding, engine restart and pool pressure. |
| `cargo test --locked -- --nocapture` (ONE full run) | Exit 0, 282 passed/2 ignored; `full.{jsonl,log}`, `full_scan.json` (zero worker failure matches). Includes the no-timeout deletion regression. |
| `scripts/lint-sizes.sh src/api/server` | Exit 0, `sizes.log`; `preservation.json` records all changed Rust sizes and unchanged fixture hashes/framing bodies. |

The captured URL pending/result matches frozen `mutations.ndjson` through the
P1-06 normalizer; Rust validation passes with optional `duplicate` present and
removed. Earlier split compile failures and the first live pressure failure
are retained (`api-unit.log`, `api_unit_v2.*`, `live_v1/`); corrected admission
preserves shutdown under pressure without weakening assertions. No release or
`all.sh` between beads, per the track policy.

Integration outstanding: P1-08's bus must consume
`Server::confirmed_mutations()` into its events topic/history with the shared
sequence policy. Direct originating/replaying delivery passes; live events-topic
delivery is not yet proven. M3 is not reached.

Continuation: P4-01 awaits P1-15; P4-02–11 and P5-02 depend transitively on the
split/API work or other tracks. P6-10 awaits P1-13 (`src/daemon/notify.rs`) and
P3-03 (`src/api/ops/shows.rs`), both absent here. No further Track D bead is
unblocked on this branch. No branch switch or merge performed.

Implementation commit: `c2269c1c49deddb42a5fd67cab9773b3a876e453` (16 files).
Metadata deviation: its message body contains literal `\n` separators, so the
co-author text is not a parsed trailer. History is preserved under the owner's
no-rewrite rule; this receipt commit has the correctly formatted trailer.
