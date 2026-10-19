# Track C — shell UI

## fbt-p3-04-0cb — IMDb service in the daemon

Landed daemon-owned lazy ratings, forced refresh/reload, typed `imdb.lookup`/`imdb` cap, bounded local-fixture HTTP reads, and atomic `<id>.jpg` posters. Additive integration: daemon field, existing torrent-dispatch fallback, typed API metadata, new IMDb transcript and IPC documentation; existing transcript bytes unchanged. All changed Rust files are below 900 lines.

| Criterion / check | Command | Result / evidence under `target/test-logs/fbt-p3-04-0cb/` |
|---|---|---|
| Two lookups load once | `daemon.py --case imdb_index_loaded_once --no-systemd` | PASS; count 1, matching cards; forced refresh changes 8.2/1234 to 9.1/4321; source-absent ratings omitted. `e2e/daemon/imdb_index_loaded_once.jsonl` |
| Local poster exists below 8 MiB | Same case | PASS; `posters/tt1234001.jpg`, 630 bytes. |
| Oversized poster refused, card retained | `daemon.py --case imdb_poster_cap --no-systemd` | PASS; 8,389,238 bytes refused, no final file/path, typed retryable upstream diagnostic; 2 MiB suggestion/search/summary caps also pass. `e2e/daemon/imdb_poster_cap.jsonl` |
| Atomic poster visibility | `daemon.py --case imdb_poster_atomic --no-systemd` | PASS; every reader size 0 or 630, final bytes match. `e2e/daemon/imdb_poster_atomic{,.reader-sizes}.jsonl` |
| Unit / contract / full | `cargo test --locked -p fbittorrent imdb:: -- --nocapture`; `cargo test --locked -p fbittorrent api:: -- --nocapture`; `cargo test --locked -- --nocapture` | PASS: 9 IMDb tests; 57 API tests, 1 ignored; full 277 passed, 2 ignored. Isolated target runner; JSONL at `imdb/regression.jsonl`, `final/api/contract.jsonl`, `final/full/all.jsonl`. `cargo check --locked --tests` also passed (`compile/check.jsonl`). |

The four real-daemon cases ran via `e2e.py` (also `api_subscriptions_deferred`), with isolated HOME/XDG and loopback HTTP. The debug build finished remotely with exit 0; after retrieval its wrapper later received SIGTERM (retained `debug_build/binary.jsonl`). The retrieved deps executable matched the remote binary SHA-256 `7b13b50e5b02e2f557b8ec7f351f8f335477da3a86eb3307962915197ac12030` and was copied immutably for all daemon tests (`fbittorrent.binary.json`). The initial strict round-trip failed on f32 rating precision; the wire conversion was corrected, and the failure remains at `api/contract.jsonl`. No release/all.sh run between beads, per track policy. Criteria unmet: none. Follow-up: slow IMDb HTTP work shares the server's existing blocking dispatch pool and request deadline; request cancellation is not added here.
