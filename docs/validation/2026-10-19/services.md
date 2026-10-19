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
