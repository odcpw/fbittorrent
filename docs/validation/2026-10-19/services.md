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
