# Track B — services

## P1-06 — fbt-p1-06-tt1

Finished the API v1 server on the P1-05d merged tree. Ruling 7 validates original live frames before excluding checking measurements from cross-time agreement; fixture agreement remains strict. Split the 2,203-line server into a 147-line façade and six production/four test modules, preserving production bodies, string literals, state declarations, public paths and test names. The largest changed source file is 523 lines (engine view); the largest server module is 451 lines. The only moved-test adjustments are its relative fixture include and P1-05d's exact checked diagnostic. A neighbouring engine fix preserves missing_files alongside the retained error after Stop/retry; real live replay and a new view regression cover it. No frozen fixture, validator, legacy wire fixture, state-table test or IPC example changed. Earlier implementation/evidence: [P1-06 history](../2026-10-18/P1-06.md); ruling 7 implementation 369a070; this section supersedes its pending-fixture qualification status.

All log paths below are relative to `target/test-logs/fbt-p1-06-tt1/`. Runs use isolated XDG directories, local payloads/peers and CARGO_BUILD_JOBS=4 through the rch shim.

| Criterion | Command run | Evidence | Observed |
|---|---|---|---|
| Live hello/errors/torrents replay | `python3 run-r8.py r8-foreground python3 run-daemon-suite.py r8-foreground target/debug/fbittorrent --no-systemd` | `r8-foreground/daemon/contract_*.jsonl` | All three green, VPN off; every non-deferred line matched |
| VPN fixture replay | `python3 run-r8.py r8-vpn-systemd python3 run-daemon-suite.py r8-vpn-systemd target/debug/fbittorrent --systemd`; focused `r8-vpn-torrents-final ... --systemd --case contract_torrents` | `r8-vpn-systemd/daemon/contract_{hello,errors}.jsonl`, `r8-vpn-torrents-final/daemon/contract_torrents.jsonl` | All three qualified, VPN on; explicit fixture0, checking, missing_files and downloading match |
| 33rd connection busy; over-cap/non-object close | Same foreground suite | `r8-foreground/daemon/{connection_limit,line_cap,non_object_line}.jsonl` | All pass with expected typed errors/closure |
| Foreign uid refused | `cargo test --locked --config 'target.x86_64-unknown-linux-gnu.runner=["env","TMPDIR=/tmp"]' -- --nocapture` | `r8-unit.log`, `api::server::tests::peercred_rejects_other_uid` | Real SO_PEERCRED owner accepted; injected foreign uid refused |
| Five-second hello deadline | Foreground suite | `r8-foreground/daemon/{hello_deadline,slow_hello_bytes}.jsonl` | Both pass; bytes do not extend deadline |
| Focused server, view, source size and move preservation | `cargo test --locked -p fbittorrent api::server:: -- --nocapture`; `cargo test --locked -p fbittorrent engine::view:: -- --nocapture`; `scripts/lint-sizes.sh --baseline scripts/size-baseline.txt src`; `python3 verify-server-split.py` | `split-server-engine.log`, `r8-view.log`, `r8-size.jsonl`, `r8-split-proof-qualified.jsonl`, `server-split-proof.json`, `server-split-moves.diff` | 17 server + 5 view tests pass, 768 rows intact; ten legacy wire fixtures unchanged; body move proof and full size ratchet pass |
| Full Rust tests | Full cargo command above, once before report | `r8-unit.{jsonl,log}` | 279 passed; 2 deliberately ignored harness entry points |
| Merged foreground/systemd coverage | Suite commands above; focused `r8-kill9-final ... --systemd --case kill9_restarts` | `r8-foreground.jsonl`, `r8-vpn-systemd/daemon/`, `r8-vpn-torrents-final.jsonl`, `r8-kill9-final.jsonl` | Foreground 39/39; initial systemd 55/57, focused retries 2/2 pass. Initial failures retained, not erased |
| Wrapped shared gate | `flock -w 7200 /tmp/fbittorrent-release.lock systemd-run --user --scope -p MemoryHigh=10G -p MemoryMax=14G --quiet -- env RUST_TEST_THREADS=4 <isolated XDG vars> bash agent_ergonomics_audit/audit/regression_tests/all.sh` | Exact argv/XDG paths `r8-all.jsonl`; `r8-all.log`; retained `r8-gate-suites/` | PASS: 279 app + 14 runtime tests; 32 scheduled cases including 20 native, no background panic/drain failures |

No acceptance criterion remains unmet and no comparison mismatch remains. Qualification limitation: the first systemd suite was not green in a single run: kill9_restarts exceeded its three-second budget (3.799s), and the final torrents Rust validation helper timed out at ten seconds while release compilation was running. Both unchanged focused cases passed after the gate. Their original failures remain in r8-vpn-systemd. Follow-up: investigate the restart-time variance and qualification-resource contention without relaxing either bound. Subscription routing/chunked delivery stays deferred to P1-08 per ruling 2; the bounded primitives and oversized response error remain tested.

<details>
<summary>Final replay normalization and itemized evidence (rulings 3–7)</summary>

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

The ruling-3/7 accepted deferred classes remain explicit: 29 capability fields, 8 undrivable catalogue frames, 1 protocol acceptance and 6 post-remove fields (44 VPN on); VPN off additionally defers torrents.ndjson:9 /result/state because explicit fixture0 binding requires the opt-in namespace. The bounded READY-window starting retries are retained with each errors case's evidence. No drivable comparison was newly deferred. These are field records, not a count of unique transcript lines.

VPN off: 45 deferred records. Values and pointers below are copied from the final successful cases.

```jsonl
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 3, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P1-18", "class": "protocol", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/proto", "reason": "proto-2 acceptance is not implemented"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 3, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 5, "pointer": "/id", "reason": "fatal malformed input has a request-less error event, not the fixture's ID-bearing failure"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 7, "pointer": "/id", "reason": "silent hello deadline has a request-less error event, not the fixture's ID-bearing failure"}
{"bead": "P1-11", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 17, "pointer": "/error", "reason": "selector ambiguity is raised by the CLI before RPC"}
{"bead": "P1-11", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 19, "pointer": "/error", "reason": "destructive-confirmation safety is raised by the CLI before RPC"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 23, "pointer": "/error", "reason": "the catalogue's unsupported legacy-only controls have no P1-06 RPC"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 25, "pointer": "/error/details/state", "reason": "ruling 4A: three READY-window retries did not produce starting refusal; engine dispatch waits for session; retry evidence target/test-logs/fbt-p1-06-tt1/r8-foreground/daemon/contract_errors.starting-retries.json"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 27, "pointer": "/error", "reason": "already_running belongs to starting a second process, not an RPC on this owner"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 29, "pointer": "/error", "reason": "the fixture's tracker-source upstream failure requires later tracker/provider operations"}
{"bead": "P1-14", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 35, "pointer": "/error/details/entries", "reason": "the fixture's diagnostic entries require the unadvertised log capability"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P5-01", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/4", "reason": "unadvertised capability peers"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/5", "reason": "unadvertised capability watch"}
{"bead": "P6-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/6", "reason": "unadvertised capability queue"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/7", "reason": "unadvertised capability origin"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P5-01", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/4", "reason": "unadvertised capability peers"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/5", "reason": "unadvertised capability watch"}
{"bead": "P6-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/6", "reason": "unadvertised capability queue"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/7", "reason": "unadvertised capability origin"}
{"bead": "ENVIRONMENT-CONDITIONAL", "class": "environment-conditional", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 9, "pointer": "/result/state", "reason": "ruling 6: explicit fixture0 binding requires the harness VPN namespace fixture (vpn.py, opt-in FBT_TEST_VPN=1)"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 13, "pointer": "/result/torrents/0/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 13, "pointer": "/result/torrents/6/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 15, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 23, "pointer": "/result/undo_until", "reason": "expected frame depends on undo/origin restoration"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 25, "pointer": "/result/files/0/ready", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 27, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 29, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 31, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 35, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 37, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 39, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 39, "pointer": "/result/torrent/hash", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 49, "pointer": "/result/torrent/undo_until", "reason": "expected frame depends on undo/origin restoration"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 51, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 59, "pointer": "/result/state", "reason": "expected asynchronous mutation state requires the unadvertised mutations capability"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 61, "pointer": "/result/state", "reason": "expected asynchronous mutation state requires the unadvertised mutations capability"}
```

VPN on: 44 deferred records. Values and pointers below are copied from the final successful cases.

```jsonl
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 3, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P1-18", "class": "protocol", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/proto", "reason": "proto-2 acceptance is not implemented"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 3, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 5, "pointer": "/id", "reason": "fatal malformed input has a request-less error event, not the fixture's ID-bearing failure"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 7, "pointer": "/id", "reason": "silent hello deadline has a request-less error event, not the fixture's ID-bearing failure"}
{"bead": "P1-11", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 17, "pointer": "/error", "reason": "selector ambiguity is raised by the CLI before RPC"}
{"bead": "P1-11", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 19, "pointer": "/error", "reason": "destructive-confirmation safety is raised by the CLI before RPC"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 23, "pointer": "/error", "reason": "the catalogue's unsupported legacy-only controls have no P1-06 RPC"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 25, "pointer": "/error/details/state", "reason": "ruling 4A: three READY-window retries did not produce starting refusal; engine dispatch waits for session; retry evidence target/test-logs/fbt-p1-06-tt1/r8-vpn-systemd/daemon/contract_errors.starting-retries.json"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 27, "pointer": "/error", "reason": "already_running belongs to starting a second process, not an RPC on this owner"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 29, "pointer": "/error", "reason": "the fixture's tracker-source upstream failure requires later tracker/provider operations"}
{"bead": "P1-14", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 35, "pointer": "/error/details/entries", "reason": "the fixture's diagnostic entries require the unadvertised log capability"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P5-01", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/4", "reason": "unadvertised capability peers"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/5", "reason": "unadvertised capability watch"}
{"bead": "P6-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/6", "reason": "unadvertised capability queue"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/7", "reason": "unadvertised capability origin"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/3", "reason": "unadvertised capability mutations"}
{"bead": "P5-01", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/4", "reason": "unadvertised capability peers"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/5", "reason": "unadvertised capability watch"}
{"bead": "P6-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/6", "reason": "unadvertised capability queue"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/7", "reason": "unadvertised capability origin"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 13, "pointer": "/result/torrents/0/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 13, "pointer": "/result/torrents/6/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 15, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 23, "pointer": "/result/undo_until", "reason": "expected frame depends on undo/origin restoration"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 25, "pointer": "/result/files/0/ready", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 27, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 29, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 31, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 35, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 37, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 39, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 39, "pointer": "/result/torrent/hash", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 49, "pointer": "/result/torrent/undo_until", "reason": "expected frame depends on undo/origin restoration"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 51, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 59, "pointer": "/result/state", "reason": "expected asynchronous mutation state requires the unadvertised mutations capability"}
{"bead": "P1-09", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 61, "pointer": "/result/state", "reason": "expected asynchronous mutation state requires the unadvertised mutations capability"}
```

VPN off: 55 extra optional field occurrences. Values and pointers below are copied from the final successful cases.

```jsonl
{"fixture": "contracts/ipc/v1/errors.ndjson", "line": 21, "live_line": 57, "pointer": "/error/details/hash", "value": "468e74471d654d237f3c100c82f44c2f962fca94"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 41, "live_line": 44, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 41, "live_line": 44, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/piece_count", "value": 65536}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/content_path", "value": "/tmp/fbt-fixture/downloads/Fixture waiting_vpn"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/size", "value": 4096}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 158, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 158, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 158, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 158, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 158, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 158, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 182, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 182, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 182, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 182, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 182, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 182, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 246, "pointer": "/result/torrent/content_path", "value": "/tmp/fbt-fixture/downloads/New transfer"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 246, "pointer": "/result/torrent/eta_s", "value": 25}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 246, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 246, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 246, "pointer": "/result/torrent/piece_count", "value": 8}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 246, "pointer": "/result/torrent/piece_size", "value": 1048576}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 246, "pointer": "/result/torrent/tracker_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 246, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
```

VPN on: 55 extra optional field occurrences. Values and pointers below are copied from the final successful cases.

```jsonl
{"fixture": "contracts/ipc/v1/errors.ndjson", "line": 21, "live_line": 57, "pointer": "/error/details/hash", "value": "cdcac4cc892898de6cf82ce50676144858c705f8"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 41, "live_line": 44, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 41, "live_line": 44, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/piece_count", "value": 65536}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/content_path", "value": "/tmp/fbt-fixture/downloads/Fixture waiting_vpn"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/size", "value": 4096}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 176, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 176, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 176, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 176, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 176, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 176, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 200, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 200, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 200, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 200, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 200, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 200, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 236, "pointer": "/result/torrent/content_path", "value": "/tmp/fbt-fixture/downloads/New transfer"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 236, "pointer": "/result/torrent/eta_s", "value": 25}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 236, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 236, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 236, "pointer": "/result/torrent/piece_count", "value": 8}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 236, "pointer": "/result/torrent/piece_size", "value": 1048576}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 236, "pointer": "/result/torrent/tracker_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 236, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
```

</details>

## P1-06 — preparation for the P1-09 merge

The orchestrator assigned structural ownership to cod_4's P1-09 (`c2269c1`). My independent split had already been committed in `d00abc2`, so a forward reconciliation commit removes those ten modules and restores the preceding 2,203-line `server.rs` layout. All ruling-7 functionality remains, including the live-frame/checking-history validation helpers and controls, daemon.py normalization, the P1-05d exact checked diagnostic assertion, and the retained missing_files reason fix/regression. Proof: server.rs equals dd2b17d's source plus only the exact checked diagnostic adaptation; daemon.py, engine/view.rs, its regression and IPC.md are byte-identical to d00abc2. Focused `cargo test --locked -p fbittorrent api::server:: -- --nocapture`: 17 passed, 1 ignored (`target/test-logs/fbt-p1-06-tt1/p109-reconcile-focused.{jsonl,log}`). Full `cargo test --locked --config 'target.x86_64-unknown-linux-gnu.runner=["env","TMPDIR=/tmp"]' -- --nocapture`: 279 passed, 2 ignored (`p109-reconcile-unit.{jsonl,log}`). Runs retain isolated XDG and CARGO_BUILD_JOBS=4. Final replay/gate qualification is pending integration of P1-09; the prior qualification above describes d00abc2, not the coming merged tree. The temporary monolith follows the explicit reconciliation instruction; the size gate is unchanged. For the merge, port ruling 7's inline argument_tests helpers/controls to cod_4's server/argument_tests.rs. Ready for the P1-09 merge.

## P1-06 — final qualification on the P1-09 split

On merge 27f8cd1, ported ruling 7's original live-validation helpers/controls into cod_4's argument_tests.rs; daemon normalization and the engine missing_files reason fix/regression survived the merge byte-identically. The P1-09 server façade and module layout are untouched; its largest child module is 467 lines (server.rs itself 561). Live replay exposed an invalid app.restart completion payload: the frozen primary owner reply is pending-only, so completed owner mutations now use outcome=done without an optional result. A new regression rejects the old done payload, round-trips the corrected event and preserves ordinary result payloads. Updated the exact capability assertion to torrents/files/mutations. Added torrents.ndjson:61 /result to the established P6-08 post-remove class: P1-09 exposes that catalogue request after its hash was removed; replay checks the actual not_found error before deferring it. This is an extension of the accepted class, not a new normalization mask. The single normalization table above and in IPC.md is unchanged; frozen fixtures/types/validators, ten legacy wire fixtures, and the 768-row state table remain intact (`r9-integrity.json`, `p109-functional-port.json`).

All evidence below is under target/test-logs/fbt-p1-06-tt1/; private XDG and CARGO_BUILD_JOBS=4 remain enforced.

| Criterion/check | Command | Log/artifact | Observed |
|---|---|---|---|
| Live fixture replay, VPN off/on | `python3 run-r9.py r9-contract-qualified python3 run-contract.py r9-contract-qualified`; same with r9-vpn-contract-qualified | `r9-contract-qualified/daemon/contract_*.jsonl`; `r9-vpn-contract-qualified/daemon/contract_*.jsonl` | All three groups pass in both modes; no remaining mismatch |
| 33rd connection busy; over-cap and non-object close | `python3 run-r9.py r9-api python3 run-daemon.py r9-api target/debug/fbittorrent <18 named cases>` | `r9-api/daemon/{connection_limit,line_cap,non_object_line}.jsonl` | All pass with typed errors and closure |
| Foreign uid refused | `cargo test --locked -p fbittorrent api::server:: -- --nocapture` | `r9-server-final.log` | Kernel same-user accepted, injected foreign uid refused; 19 server tests pass, 1 ignored |
| Hello deadline at 5 seconds | Named API batch above | `r9-api/daemon/{hello_deadline,slow_hello_bytes}.jsonl` | Both pass; trickled bytes do not reset deadline |
| Build, size and frozen preservation | `cargo check --locked`; `scripts/lint-sizes.sh --baseline scripts/size-baseline.txt src`; integrity checks | `r9-check-final.{jsonl,log}`, `r9-size.jsonl`, `r9-integrity.json` | Check and size pass; protected bytes unchanged |
| Full Rust suite | `cargo test --locked --config 'target.x86_64-unknown-linux-gnu.runner=["env","TMPDIR=/tmp"]' -- --nocapture` | `r9-unit.{jsonl,log}` | 287 passed; 2 deliberately ignored harness entry points |
| Wrapped gate | `flock -w 7200 /tmp/fbittorrent-release.lock systemd-run --user --scope -p MemoryHigh=10G -p MemoryMax=14G --quiet -- env RUST_TEST_THREADS=4 <private XDG vars> bash agent_ergonomics_audit/audit/regression_tests/all.sh` | Exact command in `r9-all.jsonl`; output `r9-all.log`; archived `r9-gate-suites/` | PASS |

Qualification history retained: r9-contract initially found the invalid owner event and line-61 post-remove mismatch; both replay modes are now green after the fixes. The named API batch was 17/18 with an obsolete two-capability assertion; the updated exact three-capability check passes in the focused r9-api-cap run. No acceptance criterion remains unmet. Remaining deferrals: 39 VPN off / 38 on, namely 22 capability-conditioned fields, 8 undrivable catalogue frames, 1 protocol acceptance, 7 post-remove fields, plus the environment-conditional fixture0 line only when VPN opt-in is off. P1-09's advertised mutation capability is actively compared. Follow-up: P1-08 must extend the exact capability expectation when it implements subscriptions; the server mutation event fix belongs on the integrated P1-09 stack.

<details>
<summary>Final deferred records and extra optional fields, copied from successful merged-stack replay</summary>

VPN off: 39 deferred records.

```jsonl
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 5, "pointer": "/id", "reason": "fatal malformed input has a request-less error event, not the fixture's ID-bearing failure"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 7, "pointer": "/id", "reason": "silent hello deadline has a request-less error event, not the fixture's ID-bearing failure"}
{"bead": "P1-11", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 17, "pointer": "/error", "reason": "selector ambiguity is raised by the CLI before RPC"}
{"bead": "P1-11", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 19, "pointer": "/error", "reason": "destructive-confirmation safety is raised by the CLI before RPC"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 23, "pointer": "/error", "reason": "the catalogue's unsupported legacy-only controls have no P1-06 RPC"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 25, "pointer": "/error/details/state", "reason": "ruling 4A: three READY-window retries did not produce starting refusal; engine dispatch waits for session; retry evidence target/test-logs/fbt-p1-06-tt1/r9-contract-qualified/daemon/contract_errors.starting-retries.json"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 27, "pointer": "/error", "reason": "already_running belongs to starting a second process, not an RPC on this owner"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 29, "pointer": "/error", "reason": "the fixture's tracker-source upstream failure requires later tracker/provider operations"}
{"bead": "P1-14", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 35, "pointer": "/error/details/entries", "reason": "the fixture's diagnostic entries require the unadvertised log capability"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-18", "class": "protocol", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/proto", "reason": "proto-2 acceptance is not implemented"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P5-01", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/4", "reason": "unadvertised capability peers"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/5", "reason": "unadvertised capability watch"}
{"bead": "P6-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/6", "reason": "unadvertised capability queue"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/7", "reason": "unadvertised capability origin"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P5-01", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/4", "reason": "unadvertised capability peers"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/5", "reason": "unadvertised capability watch"}
{"bead": "P6-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/6", "reason": "unadvertised capability queue"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/7", "reason": "unadvertised capability origin"}
{"bead": "ENVIRONMENT-CONDITIONAL", "class": "environment-conditional", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 9, "pointer": "/result/state", "reason": "ruling 6: explicit fixture0 binding requires the harness VPN namespace fixture (vpn.py, opt-in FBT_TEST_VPN=1)"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 13, "pointer": "/result/torrents/0/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 13, "pointer": "/result/torrents/6/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 15, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 23, "pointer": "/result/undo_until", "reason": "expected frame depends on undo/origin restoration"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 25, "pointer": "/result/files/0/ready", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 27, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 29, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 31, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 35, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 37, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 39, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 39, "pointer": "/result/torrent/hash", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 49, "pointer": "/result/torrent/undo_until", "reason": "expected frame depends on undo/origin restoration"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 51, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 61, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
```

VPN off: 55 extra optional field occurrences.

```jsonl
{"fixture": "contracts/ipc/v1/errors.ndjson", "line": 21, "live_line": 57, "pointer": "/error/details/hash", "value": "906fd7edee2fe50bcb23df1fd2f863efe9c2a1fb"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 41, "live_line": 44, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 41, "live_line": 44, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/piece_count", "value": 65536}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/content_path", "value": "/tmp/fbt-fixture/downloads/Fixture waiting_vpn"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/size", "value": 4096}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 258, "pointer": "/result/torrent/content_path", "value": "/tmp/fbt-fixture/downloads/New transfer"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 258, "pointer": "/result/torrent/eta_s", "value": 25}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 258, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 258, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 258, "pointer": "/result/torrent/piece_count", "value": 8}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 258, "pointer": "/result/torrent/piece_size", "value": 1048576}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 258, "pointer": "/result/torrent/tracker_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 258, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
```

VPN on: 38 deferred records.

```jsonl
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 5, "pointer": "/id", "reason": "fatal malformed input has a request-less error event, not the fixture's ID-bearing failure"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 7, "pointer": "/id", "reason": "silent hello deadline has a request-less error event, not the fixture's ID-bearing failure"}
{"bead": "P1-11", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 17, "pointer": "/error", "reason": "selector ambiguity is raised by the CLI before RPC"}
{"bead": "P1-11", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 19, "pointer": "/error", "reason": "destructive-confirmation safety is raised by the CLI before RPC"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 23, "pointer": "/error", "reason": "the catalogue's unsupported legacy-only controls have no P1-06 RPC"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 25, "pointer": "/error/details/state", "reason": "ruling 4A: three READY-window retries did not produce starting refusal; engine dispatch waits for session; retry evidence target/test-logs/fbt-p1-06-tt1/r9-vpn-contract-qualified/daemon/contract_errors.starting-retries.json"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 27, "pointer": "/error", "reason": "already_running belongs to starting a second process, not an RPC on this owner"}
{"bead": "P1-18", "class": "undrivable-catalogue-frame", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 29, "pointer": "/error", "reason": "the fixture's tracker-source upstream failure requires later tracker/provider operations"}
{"bead": "P1-14", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/errors.ndjson", "line": 35, "pointer": "/error/details/entries", "reason": "the fixture's diagnostic entries require the unadvertised log capability"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P1-18", "class": "protocol", "fixture": "contracts/ipc/v1/hello.ndjson", "line": 6, "pointer": "/result/proto", "reason": "proto-2 acceptance is not implemented"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P5-01", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/4", "reason": "unadvertised capability peers"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/5", "reason": "unadvertised capability watch"}
{"bead": "P6-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/6", "reason": "unadvertised capability queue"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 3, "pointer": "/result/caps/7", "reason": "unadvertised capability origin"}
{"bead": "P1-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/2", "reason": "unadvertised capability subscribe"}
{"bead": "P5-01", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/4", "reason": "unadvertised capability peers"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/5", "reason": "unadvertised capability watch"}
{"bead": "P6-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/6", "reason": "unadvertised capability queue"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 5, "pointer": "/result/caps/7", "reason": "unadvertised capability origin"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 13, "pointer": "/result/torrents/0/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 13, "pointer": "/result/torrents/6/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 15, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 23, "pointer": "/result/undo_until", "reason": "expected frame depends on undo/origin restoration"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 25, "pointer": "/result/files/0/ready", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 27, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 29, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 31, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 35, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 37, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 39, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 39, "pointer": "/result/torrent/hash", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
{"bead": "P6-08", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 49, "pointer": "/result/torrent/undo_until", "reason": "expected frame depends on undo/origin restoration"}
{"bead": "P5-05", "class": "capability-conditioned", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 51, "pointer": "/result/torrent/playable", "reason": "expected frame depends on the unadvertised watch capability"}
{"bead": "P6-08", "class": "post-remove", "fixture": "contracts/ipc/v1/torrents.ndjson", "line": 61, "pointer": "/result", "reason": "ruling 4E: operation after preceding remove requires a surviving/undo record"}
```

VPN on: 55 extra optional field occurrences.

```jsonl
{"fixture": "contracts/ipc/v1/errors.ndjson", "line": 21, "live_line": 57, "pointer": "/error/details/hash", "value": "ba9696954bdd3b58d87f551cf8b94deb057cbc30"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 41, "live_line": 44, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 41, "live_line": 44, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/piece_count", "value": 65536}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 43, "live_line": 50, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 45, "live_line": 84, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/ratio", "value": 0.0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 47, "live_line": 90, "pointer": "/result/torrent/uploaded_bytes", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/content_path", "value": "/tmp/fbt-fixture/downloads/Fixture waiting_vpn"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/size", "value": 4096}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 53, "live_line": 106, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 55, "live_line": 179, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/piece_count", "value": 4}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/piece_size", "value": 1024}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/tracker_count", "value": 0}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 57, "live_line": 203, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 270, "pointer": "/result/torrent/content_path", "value": "/tmp/fbt-fixture/downloads/New transfer"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 270, "pointer": "/result/torrent/eta_s", "value": 25}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 270, "pointer": "/result/torrent/file_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 270, "pointer": "/result/torrent/origin", "value": "manual"}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 270, "pointer": "/result/torrent/piece_count", "value": 8}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 270, "pointer": "/result/torrent/piece_size", "value": 1048576}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 270, "pointer": "/result/torrent/tracker_count", "value": 1}
{"fixture": "contracts/ipc/v1/torrents.ndjson", "line": 63, "live_line": 270, "pointer": "/result/torrent/upload_limit_bps", "value": 32768}
```

</details>


## P7-09 — fbt-p7-09-duv — API/engine checkpoint

Implemented durable parked adds for v1 magnets and local metainfo, the `deferred_add` capability, the reserved `add.deferred` result field and a new six-line contract fixture. New records append to the ledger, preserve run/pause, seed, tracker and file-selection intent, retain private store permissions, and survive daemon restart. URL sources are refused before fetching or acknowledging pending. When the isolated fixture interface appears, real BEP 9 metadata resolves and payload starts; local metainfo with explicit pause is checked without starting and retains its file selection. Admission clears the deferred marker. Existing false markers are omitted from serialized records; legacy response bytes and prior fixtures stay unchanged. Neighbouring changes only add the marker's false initializer in existing Record literals and suppress pending magnet confirmation for a completed deferred add. No source file changed here exceeds 682 lines.

Evidence root: `target/test-logs/fbt-p7-09-duv/`. The retained `run.py` sets CARGO_BUILD_JOBS=4, rch PATH and isolated XDG for Rust commands; daemon cases use the shared private Sandbox and real loopback/namespace fixtures.

| Criterion / step | Command | Log / artifact | Observed |
|---|---|---|---|
| Missing interface returns deferred | `python3 run.py saved-record-final python3 run-daemon.py saved-record-final target/debug/fbittorrent deferred_add_saved_record` | `saved-record-final.jsonl`, `saved-record-final/daemon/deferred_add_saved_record.jsonl` | PASS: magnet and selected local metainfo reply done/deferred; waiting_vpn; duplicate preserves exact ledger bytes; URL makes no HTTP request; daemon restart preserves ledger bytes and intent |
| Interface appears, metadata resolves, transfer starts | `python3 run.py vpn-deferred-final python3 run-daemon.py vpn-deferred-final target/debug/fbittorrent deferred_add` (`FBT_TEST_VPN=1` set by wrapper) | `vpn-deferred-final.jsonl`, `vpn-deferred-final/daemon/deferred_add.jsonl` | PASS: real isolated namespace; timestamped fixture0 appearance; waiting_vpn → metadata → downloading with positive rate; paused local metainfo checked with original file selection |
| Durable magnet and queue order | `cargo test --locked -p fbittorrent engine::ledger::tests:: -- --nocapture` | `ledger-unit.{jsonl,log}` | 3 pass, including deferred_record_from_magnet; saved btih/dn/tracker, end-of-ledger, generation and duplicate-byte checks |
| New fixture / frozen decoder | `cargo test --locked -p fbittorrent api::ops::torrents::deferred_add_fixture_roundtrips -- --nocapture` | `api-fixture.{jsonl,log}`, `contracts/ipc/v1/deferred-add.ndjson` | 1 pass; old fixtures unchanged |
| Existing live add / mutation routes | `python3 run.py add-regressions python3 run-daemon.py add-regressions target/debug/fbittorrent api_torrent_operations api_concurrent_adds api_subscriptions_deferred mutation_magnets_and_file_removal` | `add-regressions.jsonl`, `add-regressions/daemon/*.jsonl` | All 4 cases pass, including concurrent duplicate identity and pending live magnets |
| Compile / source ceiling | `cargo check --locked`; `bash scripts/lint-sizes.sh --baseline scripts/size-baseline.txt src` | `check.{jsonl,log}`, `source-size.{jsonl,log}` | Both pass; debug build also passes |
| Full Rust suite, once | `cargo test --locked --config 'target.x86_64-unknown-linux-gnu.runner=["env","TMPDIR=/tmp"]' -- --nocapture` | `full-unit.{jsonl,log}` | 288 pass, 2 ignored, 1 failure: unchanged native_runtime::shutdown_tests::saturated_stderr_does_not_extend_shutdown_or_erase_phase_errors killed its child after 10 seconds |
| Failure qualification only | Same cargo command filtered to `native_runtime::shutdown_tests::saturated_stderr_does_not_extend_shutdown_or_erase_phase_errors` | `shutdown-focused.{jsonl,log}` | PASS immediately, 0.324 seconds for command; original full failure retained |
| CLI deferred JSON and patch bookkeeping | Not run: CLI not changed | Existing ownership question pending | UNMET: src/main.rs is 1,360 lines and shared with Track A/P1-11. Structural split ownership was requested per owner policy; CLI stays on legacy refusal until API client wiring is coordinated. No CLI field/version claim is made |

This is a coherent API/engine checkpoint, **not a completed P7-09**. No second all.sh/release build was run between track beads, per owner track policy; P1-06's final wrapped gate is retained in the preceding section. Initial namespace failure used a private seed, which correctly refused BEP 9; corrected to a public torrent inside the loopback-only owned namespace, and fixed shutdown's missing args. Initial restart-test failures were receiver lifecycle mistakes; final case closes/recreates its owned notify receiver and passes. All original logs remain. Follow-ups: coordinate the CLI field and contract patch with Track A; investigate the unchanged saturated-stderr test's full-suite-only timeout without relaxing its bound. P3-01 and the remaining earlier track beads still wait on unmerged P1-08/client/UI prerequisites; P7-01 waits on P3-01.


## P1-11 — fbt-p1-11-zqb — CLI/client checkpoint

The native CLI now uses a bounded NDJSON client (200 ms hello deadline, eight correlated requests, separate bounded event queue, one reader joined on close), atomic add results, 256-hash batches, normalized-to-1.x shapes, offline reads and lock-protected stop/delete intent, origin attribution, clap-derived capabilities 1.2.0, warning envelopes, deterministic JSON and API shutdown/restart. Idempotent interrupted requests retain their ref and observed instance; removal is never replayed and reports timeout/unknown outcome. Subscription decoding uses the frozen revision tracker and a bounded reconnect backoff. RSS/Shows add callers use API v1 when present; settings retains its authorized compatibility path. The merged tree already contained the isomorphic main.rs split (0643b16); this change builds on it, preserving the existing TUI/golden tests. Every changed source is below 900 lines (main.rs 791, native CLI 714). Integration repairs are minimal: close ImdbCard's missing brace, union the overwritten hello caps, combine the two contradictory merged cap assertions, and share the build-ID override needed for skew qualification. Prior fixture lines, legacy wire fixtures and the 768-row state table are unchanged.

Evidence root `target/test-logs/fbt-p1-11-zqb/`: debug build `build-offline.jsonl`; full `cargo test --locked --config 'target.x86_64-unknown-linux-gnu.runner=["env","TMPDIR=/tmp"]' -- --nocapture` once, `full-unit.{jsonl,log}`: **315 passed, 2 ignored**, including TUI goldens and frozen API/engine fixtures. `client-unit-final.{jsonl,log}`: seven client tests pass (eight out-of-order requests, event separation, fixed reconnect schedule, frozen subscription transcript/gap rejection, same-ref add resend). Initial compiler/test/harness failures remain in their original logs. No all.sh/release build was run between track beads, per the owner track policy.

| Acceptance / named coverage | Command | Evidence relative to root | Observed |
|---|---|---|---|
| Agent contract and 1.2.0 changes | `python3 agent_contract.py target/debug/fbittorrent --log-root .../live-final`; same with `--systemd` | `live-final/agent_contract/*.jsonl`, `live-systemd/agent_contract/*.jsonl`, `exits-final.jsonl` | All seven cases green in foreground and systemd: original R-001…R-012 assertions, exits 1…8 (including real 33rd-connection busy and local failed IMDb endpoint), origin, deferred CLI output, warnings, no-start and determinism |
| Reads never start; offline ledger intents survive drain | Three named `daemon.py::offline_*` cases; `native.py::launcher_never_called` | `offline-qualified/daemon/{offline_stop_ledger,offline_delete_keep_files_ledger}.jsonl`, `offline-drain-final/daemon/offline_write_waits_for_draining_daemon.jsonl`, `launcher-final/native/launcher_never_called.jsonl` | PASS: running:false, zero launcher calls, pause/selection/order preserved, metainfo removed while payload retained, concurrent down+stop loses no final save |
| Determinism and JSON stdout | `agent_contract.py::json_determinism`; `scripts/verify-determinism.sh` invokes the same case | `live-final/agent_contract/json_determinism.jsonl` | Seven verbs each match SHA256/bytes across both SOURCE_DATE_EPOCH runs; ledger timestamps retained; diagnostics separate |
| list entries from normalized A.9 | `cli::shapes::tests::unknown_values_preserve_legacy_sentinels_only`; live add/list/show cases | `full-unit.log`, `live-final/agent_contract/contract.jsonl` | PASS for frozen waiting_vpn shape and live data; raw `api` verb equality remains assigned to P1-18 |
| Source ceiling | `bash scripts/lint-sizes.sh --baseline scripts/size-baseline.txt src` | `size-final.{jsonl,log}` | PASS; no source exceeds its ceiling |

**P1-11 remains in progress**: the branch lacks P1-08, P1-12, P1-14 and P1-17. Live summary subscription/presence, idle exit, usage/log integration, and wait across an upgrade/idle exit cannot yet be qualified. Status uses Summary.daemon when the subscribe cap exists; until that integration it reports the available state/upgrade field without invented idle counters or busy reasons. Cumulative app.info client counters are not used for presence. The existing TUI-hosted path remains for P1-15: v0.1-style hosts permit compatibility reads and refuse writes with the quit-Q hint. No complete native TUI suite or final integrated gate is claimed here. Follow-ups: qualify those owner-dependent cases after merges; preserve actual category/tag values if a later normalized contract exposes them (current A.9 lacks those legacy fields). P7-09's CLI field works in the live offline_records case; its patch-version bookkeeping follows separately.


## P7-09 — fbt-p7-09-duv — completed CLI/contract integration

Completes the earlier API/engine checkpoint 4a61554 using the real CLI path landed in P1-11 checkpoint 6cb936e. `add --json` retains the API's atomic hash, duplicate result and `deferred:true`; human output says waiting for VPN. Capabilities now advertise the shared server capability list, including deferred_add, and contract **1.2.1** records `add.deferred` while preserving the 1.2.0 status break and legacy sentinel notes. The engine, durable marker, queue ordering and frozen pre-existing fixtures are unchanged in this completion commit.

| Acceptance / check | Command | Evidence under `target/test-logs/fbt-p7-09-duv/` | Result |
|---|---|---|---|
| Missing interface: add answers deferred | `agent_contract.py ... --case contract --case offline_records` | `cli-final/agent_contract/offline_records.jsonl` | PASS through the actual CLI, missing fbmissing0; returned hash/deferred, saved record, offline pause and keep-files deletion |
| Interface appears: metadata resolves and torrent starts | `FBT_TEST_VPN=1 daemon.py::deferred_add` through retained run.py/run-daemon.py | `vpn-cli-final/daemon/deferred_add.jsonl` | PASS with real public BEP 9 seed inside the owned loopback-only namespace; timestamped fixture0 appearance, metadata then positive-rate downloading; stopped local metadata and selection preserved |
| Contract patch / CLI regressions | `cargo test --locked -p fbittorrent agent:: -- --nocapture`; real contract case | `cli-contract-unit.{jsonl,log}`, `cli-final/agent_contract/contract.jsonl` | Five agent unit tests and CLI round trip pass; version 1.2.1 and add.deferred change present |
| Full Rust suite once for completion | `cargo test --locked --config 'target.x86_64-unknown-linux-gnu.runner=["env","TMPDIR=/tmp"]' -- --nocapture` | `full-cli-unit.{jsonl,log}` | PASS: 315 passed, 2 ignored |

Both acceptance criteria and the previously missing CLI/patch steps are met. No all.sh/release build between track beads, per owner track policy. The old full-suite saturated-stderr failure from the initial checkpoint remains retained; the current full suite is green. Namespace evidence proves this isolated interface, not the owner's real proton0 configuration. Follow-ups are the separate P1-11 bus/idle/upgrade integration items already recorded above.


## P3-05 — fbt-p3-05-8q4 (settings service checkpoint)

Added typed `app.settings.get/set`, the `settings` capability/table rows and a new independent settings transcript; all nine pre-existing transcripts are byte-identical (`target/test-logs/fbt-p3-05-8q4/frozen-fixtures.json`). Settings validate every key before saving, preserve config bytes on an invalid patch, apply queue/session limiters and add defaults live, and request the existing supervisor for encryption/binding/port changes. CLI settings use v1 while running and the owner lock/private config while down, expose canonical values with contract 1.2.2, and never start a daemon to save settings. The configured open command now receives flags and a literal target argument. **Owner ruling:** settings changes are not pending mutations, so no `mutation.result` follows: a restart-class change answers `{applied: [keys], restart: true}` after the durable save, the supervisor restarts the engine slot, and clients observe completion through `engine.changed` (and the `network` state when the binding changed). Documented in IPC.md. Neighbouring additions cover config defaults/validation, daemon live-policy application/atomic idle policy, session limiters, and API add defaults/CLI dispatch; lock/lifecycle ownership is unchanged. P1-08 is absent on this branch: pushed settings state and the two engine.changed frames remain **unmet pending P1-08 integration**; a daemon-owned generation and typed snapshot are available for its producer. P1-12 consumes the atomic idle setting; notification/handler/clipboard consumers remain with their assigned beads. No all.sh between track beads per the owner policy.

All paths below are under `target/test-logs/fbt-p3-05-8q4/`. `run.py` retains exact commands and exit codes; process cases use the retaining Sandbox, private HOME/TMPDIR/five XDG directories and disposable transient units.

| Criterion/check | Command | Evidence | Observed |
|---|---|---|---|
| Invalid value names key | `cargo test --locked -p fbittorrent settings:: -- --nocapture`; `daemon.py --case settings_invalid_named` (foreground and systemd through run-daemon.py) | `settings-unit.{jsonl,log}`; `foreground-final/daemon/settings_invalid_named.jsonl`; `systemd-final/daemon/settings_invalid_named.jsonl` | PASS: every writable key plus unknown/read-only keys; rejected multi-key patch preserves both saved bytes and effective settings |
| Queue limit applies live | `run-daemon.py {foreground-final,systemd-final} target/debug/fbittorrent settings_live_queue` | corresponding `daemon/settings_live_queue.jsonl`; full-unit-final log's `live_settings_replace_session_limiters_without_restarting` | PASS: two real checked incomplete torrents, active/queued 1/1 → 2/0 → 1/1 via CLI; same engine epoch; new seed/save defaults; same session primary→alternative→unlimited limiters |
| Restart keeps clients | `run-daemon.py {foreground-final,systemd-final} target/debug/fbittorrent settings_restart_class_keeps_clients` | corresponding `daemon/settings_restart_class_keeps_clients.jsonl` | PASS for durable ack, real Running→Starting→Running, same API connection/instance/PID, epoch +1, identical repeat no-op, no mutation.result. Pushed state/engine.changed verification pending P1-08 |
| Offline CLI and contract | `run-daemon.py foreground-final target/debug/fbittorrent settings_offline_no_start`; `agent_contract.py target/debug/fbittorrent --case contract --log-root .../cli-contract-final` | `foreground-final/daemon/settings_offline_no_start.jsonl`; `cli-contract-final/agent_contract/contract.jsonl` | PASS: saved options, invalid value exit 2, no manager invocation or live socket; patch 1.2.2 additive bookkeeping |
| Full unit suite | `cargo test --locked --config 'target.x86_64-unknown-linux-gnu.runner=["env","TMPDIR=/tmp"]' -- --nocapture` | `full-unit-final.{jsonl,log}` | 320 passed, 2 deliberately ignored; original full-unit failure retained: new patch had accepted unknown args; strict deserializer fixed, existing rejection test unchanged |
| Configured player | `cargo test --locked -p fbittorrent config::tests::open_command_keeps_arguments_and_literal_target -- --nocapture` | `open-command-unit.{jsonl,log}` | PASS, added after full suite: exact flag plus one target containing spaces, semicolon and command-substitution text |
| Build/size | `cargo check --locked`; `cargo build --locked`; `scripts/lint-sizes.sh --baseline scripts/size-baseline.txt src`; `git diff --check` | `check.{jsonl,log}`; `debug-binary-final.{jsonl,log}`; `size-final.{jsonl,log}` | PASS; every changed source below 900 lines |

Follow-up: when P1-08 lands, publish Service::generation/get as Settings state-on-change, then qualify pushed settings and both engine.changed events (including a binding change); when P1-12 lands, qualify actual idle expiry after changing idle_exit_secs. P3-06 requires P1-12 and has not been started on this checkpoint.


## P5-03 — fbt-p5-03-5ry (daemon/API half)

Added `torrents.recheck`, its pending/ref and measured result types, `recheck` capability, non-idempotent operation row, independent success/failure transcript and contract 1.2.3. Engine-owned work checks one hash at a time outside the dispatch pool and releases the ledger before waiting for checksum I/O. Duplicate requests share a check and result; admission/handle replacement serializes against ledger mutations, paused intent stays paused, checks consume no queue slots, and owner teardown cancels/joins work and publishes terminal receipts before bye. Saved completion is invalidated before opening data; missing/failed checks cannot resurrect the old finished claim, while unchanged verified data retains its original completion time. Added a serde-default, omitted-when-false staged bit solely to support the required refusal before P3-06. No previous transcript or legacy byte fixture was regenerated. UI criteria (R action, default-No confirmation, overview button/checking presentation, keymap/UI tests) are **deferred to tracks C/D** per the owner clarification. The CLI verb remains a track-B surface follow-up outside this explicitly assigned daemon/API half. Completion notification re-emission and summary mutation busy-reason qualification await P1-08/P1-12/P1-13; durable reset/second completion and the server's pending mutation registry are proven here. No all.sh between track beads per owner policy.

Evidence root: `target/test-logs/fbt-p5-03-5ry/`; exact argv/exit codes live in its command JSONL files. Named process cases run over api.sock with private HOME/TMPDIR/five XDG dirs, loopback peers and disposable systemd units.

| Criterion / check | Command | Log / artifact | Observed |
|---|---|---|---|
| Corrupt-byte verification then repair | `run-daemon.py repair-qualified target/debug/fbittorrent recheck_corrupt_byte recheck_corrupt_byte_256k` | `repair-qualified/daemon/recheck_corrupt_byte{,_256k}.jsonl` | PASS: seed stays paused until mutation.result; 64 MiB case observes checking→stalled/downloading→done; exact 256 KiB/16-piece case reports 245760 verified bytes, then local seed repairs the missing piece and restores exact SHA256/completion. A 256 KiB scan can finish between API reads, so the larger independent fixture proves observable state without adding production delays; the prescribed 256 KiB fixture remains an exact-byte test |
| Dormant check once | `cargo test --locked -p fbittorrent recheck -- --nocapture`; `run-daemon.py live-final ... recheck_dormant` | `module-diagnostic.{jsonl,log}`; `live-final/daemon/recheck_dormant.jsonl` | PASS: dormant row checked:false; one storage create, one 5-byte checksum read; one real admission in daemon log |
| Paused intent preserved | `run-daemon.py live-final ... recheck_paused_stays_paused` (also systemd-final) | corresponding `daemon/recheck_paused_stays_paused.jsonl` | PASS: verifies complete data while saved paused:true/intent:pause remain; final paused; metadata-less and unknown hashes refused |
| Mid-repair restart | `run-daemon.py {live-final,systemd-qualified} ... recheck_resets_completion` | corresponding `daemon/recheck_resets_completion.jsonl` | PASS: SIGKILL during an API-observed check leaves completion_on=0; restart hashes and returns incomplete/stalled, never dormant done |
| Staged refusal | `run-daemon.py {live-final,systemd-final} ... recheck_staged_invalid_state` | corresponding case JSONL | PASS: isolated saved staged record yields invalid_state with reason:staged (A.3 exit mapping 1); actual staging operation/CLI case awaits P3-06/CLI surface |
| Failure is terminal and truthful | `run-daemon.py {live-final,systemd-qualified} ... recheck_failed_receipt`; focused recheck units | corresponding case JSONL; `module-diagnostic.log` | PASS: deliberate cache-write failure gives one failed ref event with per-hash message, error row, durable completion 0; reconciliation cannot restore the old completion |
| Async sharing / no timeout | `cargo test --locked -p fbittorrent recheck -- --nocapture` | `module-diagnostic.{jsonl,log}` | 10 passed, including gated real checksum read: two requests, one physical scan; info/reconcile responsive; zero queue slots; pending receipt remains valid beyond its nominal deadline; operation non-idempotent |
| Full suite / compatibility | `cargo test --locked --config 'target.x86_64-unknown-linux-gnu.runner=["env","TMPDIR=/tmp"]' -- --nocapture` | `full-unit.{jsonl,log}`; final focused log above | Full run 327 passed, 2 deliberately ignored; subsequent admission/failure/timestamp hardening covered by final 10 focused tests. Ten pre-existing transcripts byte-identical (`frozen-fixtures.json`); 768-row mapping and old wire goldens passed full run |
| Contract / source size | `agent_contract.py ... --case contract`; `scripts/lint-sizes.sh --baseline scripts/size-baseline.txt src`; `git diff --check` | `cli-contract/agent_contract/contract.jsonl`; `size.{jsonl,log}` | PASS; existing CLI commands unchanged; all changed/new source files below 900 lines |

Initial failures retained: first restart/repair fixtures waited for visible done before its timestamp had been durably saved; corrected to wait for positive completed_at. The systemd-run SIGKILL launcher status is 255, versus foreground -9 (earlier wrong 137 expectation retained). The initial 256 KiB checking observation was too short for sampling; the separate 64 MiB state fixture above supplies that assertion. One parallel focused run missed the checksum read gate's existing 5-second deadline; unchanged exact test passed in 37 ms and the final 10-test module passed with the same deadline/assertions plus better diagnostics. Follow-up: investigate that gate scheduling variance; replace the saved staged-record precondition with real `stage:true` after P3-06; qualify completion pushes and busy-reason integration after track A lands; wire the CLI verb on track B and UI criteria on C/D.
