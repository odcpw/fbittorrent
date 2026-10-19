# 2026-10-18: landing the native product (P0-01 to P0-04)

Receipts in the plan's Appendix D form. Toolchain: the rustup Rust 1.98.0
(`~/.rustup/toolchains/1.98.0-x86_64-unknown-linux-gnu/bin/cargo`). Logs under
`target/test-logs/` are local and not in git.

## P0-01

Owner decision Q4 (2026-10-18): "no archive; data deleted, README summaries kept
in git". This replaces the plan's archive, manifest and `ARCHIVE.md`.

- Whole-tree backup before any change: `tar --zstd -cf
  worktree-2026-10-18.tar.zst --exclude=./target .` (exit 0; 101,474,414 bytes;
  SHA256 `07acf5cb38581ba909b8ce3ddb53b44386e39875538db0aa56cc0220cb5a46a2`;
  6,129 entries, equal to the file count of the tree). Temporary under Q4:
  deleted once the landing commits existed.
- Before: `docs/validation/` held 2,197 files in 158 directories, 286,491,746
  bytes (`du -sh`: 279M), six dated directories 2026-10-09 to 2026-10-14.
- Kept: the 18 `README.md` files under `docs/validation/` (152K; the largest is
  14,980 bytes; nothing over 200 KB). Four of them are copies of the nova3
  runtime README inside the 2026-10-13 `.work` directories.
- Deleted: the other 2,179 files, including four identical copies of IMDb's
  `title.ratings.tsv` (30,052,145 bytes each). An archive was first built and
  verified (2,161 files, every hash matched) and then deleted with
  `~/Archive/fbittorrent/` on the owner's decision; nothing of it remains.
- `scripts/check-doc-links.sh`: exit 0; 178 links into `docs/validation/`
  checked, 0 broken, 167 to deleted receipt data (listed, not failed); log
  `target/test-logs/docs/links.jsonl`. From the docs: `docs/tui-ux-review.md:14`
  and `docs/queue-encryption-vpn-plan.md:5` link
  `validation/2026-10-10/README.md` (found); `docs/engine-evaluation.md:119-120`
  name `docs/validation/2026-10-09/README.md` and
  `docs/validation/2026-10-10/README.md` (both exist);
  `docs/tui-ux-review.md:13` links `validation/2026-10-10/native-integration.txt`,
  which is deleted data and was left as it is.

## P0-02

Option B (owner decision): the full vendored tree, no public fork.

- Commit C0 `3f3eccc` adds `vendor/asupersync/` (974 files; git tree
  `5d925d43dd9e48cecb14a5bc022794ad84aae88c`; SHA256 of the sorted
  `sha256sum` list of its files
  `4e3fa9c5c11ba5e19849c5e14ceb6141e00763fbbec3b88b12e34d40638a678a`).
  `Cargo.toml` keeps `[patch.crates-io] asupersync = { path = "vendor/asupersync" }`.
- Source: `asupersync-0.5.0.crate`, sha256
  `f34b1a19ffd6b74570339a156912436335bb09c594c675c62ea164b19a1f2511` (matches the
  plan). `diff -rq --exclude=.cargo-ok` against the registry copy lists exactly
  the ten patched files plus `FBITTORRENT_PATCH.md`, which was rewritten to list
  them. `cmp` of `LICENSE` against the registry copy: identical.
- The patch's own tests could not be run in place. `cargo test --all-features
  --offline --locked --manifest-path vendor/asupersync/Cargo.toml` (in a
  byte-identical copy) stops at dependency resolution (`addr2line v0.25.1` is
  not in the local registry; `--all-features` also enables the nightly-only
  `nightly-outcome-try` and compiles `region_owner_tests` out, since they need
  `not(feature = "messaging-fabric")`). With the app's features plus
  `epoch-diagnostics`, `cargo test --lib --no-run` fails because the crates.io
  package omits test-only module files that `#[cfg(test)]` declarations name
  (`src/actor_conformance_tests.rs` first; stubbing 94 of them in the copy left
  227 errors in about 30 files). Counts: none. The last run of the wheel tests is
  the retired runner's 63 passed on 2026-10-11 (summary in
  `docs/validation/2026-10-11/cpu-optimization/README.md`; its receipt data was
  deleted under Q4). The patched code is exercised by the app suites below.
- Retired: `timer_wheel_tests.rs` and `run_timer_wheel_tests.py` deleted from
  the tree; absent from C5 `7de96d2`.
- Clean clone, no network: `cargo build --release --locked --offline` exit 0 in
  7 min 21 s (see P0-04).

## P0-04

Committed directly on `main` by the owner's instruction (no pull request and no
merge commit). `codex/shows-native-engine` pointed at the same commit as `main`
(`564fa18`) and is deleted once `main` is pushed; the tag `v0.1.1` goes on the
final landing commit (the beads commit that follows this receipt).

| Commit | Message |
|---|---|
| `3f3eccc` | C0 vendor: asupersync 0.5.0 with the fbittorrent patch |
| `47fa008` | C1 vendor: rqbit 9.0.1 fork ported to Asupersync |
| `2b6f15d` | C2 vendor: nova3 search runtime |
| `ff7dbe2` | C3 app: native engine, IPC, VPN binding, queue, encryption, Shows, RSS, search, TUI |
| `8181475` | C4 plugin: bar widget on the native app |
| `7de96d2` | C5 tests: native integration, engine benchmarks |
| `c89a4ec` | C6 docs: engine evaluation, queue and VPN plan, TUI review, validation receipts, 2026-10-18 plan |

Clean clone of `main` at `c89a4ec` (log `target/test-logs/land/clone.jsonl`,
7 steps, 0 failed):

| Step | Result |
|---|---|
| `git clone` of the repository | exit 0 |
| `git status --porcelain` in the clone | empty |
| `cargo build --release --locked --offline` | exit 0, 7 min 21 s |
| `agent_ergonomics_audit/audit/regression_tests/all.sh` | exit 0, 3 min 48 s: hygiene checks (6 paths, 6 nova3 rows); `cargo test --locked -p fbittorrent -p rqbit-runtime`: 90 passed, then 14 passed and 1 ignored, then 0; no worker-panic match; release build; `native.py`: 11 checks ok, "native integration passed" |
| `capabilities --json`, clean-clone build vs `~/.local/bin/fbittorrent` (21,555,840 bytes, built 2026-10-14) | `cmp` exit 0; both 7,542 bytes, SHA256 `603331c7f1facf0e7bb57b3d216c68bba9eafce1376662cae4e1f87eed83a5bb` |

Secret scan (P0-03 step 4) over the committed tree and `.beads/`:
`git grep -n -I -i -E "api[_-]?key|token|password|BEGIN .*PRIVATE KEY"`, 8,303
hits, all reviewed. By area: `vendor/asupersync` 7,922, `vendor/rqbit` 148,
`.beads` 107, `docs/plan-2026-10-18` 78, `vendor/nova3` 29, `src` 17,
`docs/validation` 2. No secret found:

- `src` (17): identifiers only, `password: Option<String>` for the legacy WebUI
  login (`src/config.rs:24,56`, `src/qbt.rs:254-285`, never a value),
  `cancellation_token()` calls, a token-bucket message, the test URL
  `https://example.org/payload.torrent?token=fixture` (`src/search.rs:1122`).
- `vendor/rqbit`, `vendor/nova3`: DHT tokens, cancellation tokens, SOCKS and
  HTTP username/password parameters.
- `vendor/asupersync`: code identifiers, plus upstream test fixtures: PEM test
  keys in `tests/atp_quic_bounded_memory.rs:47`,
  `src/net/quic_native/handshake_driver.rs:1569`, `src/tls/acceptor.rs:1292`,
  truncated or malformed PEM strings in `src/tls/types.rs:839-904`, redaction
  test inputs in `src/atp/logging/redaction.rs`; all published in the crates.io
  package.
- Docs and `.beads`: prose (theme tokens, `--no-ask-password`, Torznab
  `api_key_file`, the scan command itself).
- No files named like keys or credentials (`.env`, `*.pem`, `*.key`, `id_*`);
  no provider-format tokens (`ghp_`, `github_pat_`, `sk-`, `xox?-`, `AKIA`,
  `AIza`).

## P0-12

Receipt: [P0-12.md](P0-12.md) (branch ntm/fbittorrent/cod_3, commit 4a07691; merged by the orchestrator after an independent test run).

## P0-16

Receipt: [P0-16.md](P0-16.md) (branch ntm/fbittorrent/cod_2, commit fe18858; merged by the orchestrator after an independent test run and an Astra review).

## P0-13

Receipt: [P0-13.md](P0-13.md) (branch ntm/fbittorrent/cod_2, commit ea13ef3; merged by the orchestrator after an independent test run).

## P0-05

Receipt: [P0-05.md](P0-05.md) (branch ntm/fbittorrent/cod_4, commit d4eb8fa; merged by the orchestrator after an independent test run).

## P0-02

Receipt: [P0-02.md](P0-02.md) (branch ntm/fbittorrent/cod_3, commit cf92a57; merged by the orchestrator after re-running the 63 timer-wheel tests in the prepared upstream checkout).

## Incident: swarm killed by systemd-oomd

Report: [incident-tmux-death.md](incident-tmux-death.md) (orchestrator investigation, 2026-10-18).

## P0-17

Receipt: [P0-17.md](P0-17.md) (branch ntm/fbittorrent/cod_4, commit da37f93; merged by the orchestrator after five full-suite runs and an Astra review).

## P0-15

Receipt: [P0-15.md](P0-15.md) (branch ntm/fbittorrent/cod_1, commits 28f1244..acb0953; merged by the orchestrator after a green full gate and an Astra review).

## P0-10

Receipt: [P0-10.md](P0-10.md) (branch ntm/fbittorrent/cod_2, commits 842b15b..80d7ad9; merged by the orchestrator after a green full gate and an Astra review).

## P0-11

Receipt: [P0-11.md](P0-11.md) (branch ntm/fbittorrent/cod_4, commits a894543..4b3515c; merged by the orchestrator after an Astra review; batch gate on main follows).

## P0-07

Receipt: [P0-07.md](P0-07.md) (branch ntm/fbittorrent/cod_3, commits 10672b7..87b9dcf; merged by the orchestrator after an Astra review).

## P0-06

Receipt: [P0-06.md](P0-06.md) (branch ntm/fbittorrent/cod_1, commits 15d73df..59ba0f4; merged by the orchestrator after an Astra review; hosted CI run checked after the push).

## P1-07 (harness half)

Receipt: [P1-07.md](P1-07.md) (branch ntm/fbittorrent/cod_1, commits 64d131b..846d05d; merged by the orchestrator after an Astra review; the fixture-serving half waits for P1-06).

## P1-01

Receipt: [P1-01.md](P1-01.md) (branch ntm/fbittorrent/cod_2, commit f3e03eb; merged by the orchestrator after an Astra review; batch gate on main follows).

## P1-02

Receipt: [P1-02.md](P1-02.md) (branch ntm/fbittorrent/cod_2, commit 4d0880b, merge 00bb354; merged by the orchestrator after an Astra review that independently ran the post-split test binary: 173/173, golden 2,704 bytes exact, size gate 15 files / max 702 lines; batch gate on main follows).

## P6-12

Receipt: [P6-12.md](P6-12.md) (branch ntm/fbittorrent/cod_3-p6-12-hn4, commits f38f7fa and 5e753b1; merged by the orchestrator after two Astra reviews, the first returning three findings on peer and tracker provenance that the second verified closed by independent runs of the four private-torrent tests; the Overview "private" marker criterion lives in P6-12b; batch gate on main follows).

## P1-19

Receipt: [P1-19.md](P1-19.md) (branch ntm/fbittorrent/cod_2, commits 9e60087 and 57f2832; merged by the orchestrator after two Astra reviews, the first returning two findings (a wildcard arm in the state derivation, changed member order in parked/queue/settings responses) that the second verified closed by an independent engine-test run with the ten frozen old-code wire fixtures; batch gate on main follows).

## P1-03

Receipt: [P1-03.md](P1-03.md) (branch ntm/fbittorrent/cod_1, commits e320b68 and 8a1d4dd with the orchestrator's reconciliation merge a3c5fb3; merged by the orchestrator after two Astra reviews, the first returning two P2 findings (agent documentation, drain-failure log format), one P3 (plain second-owner hint) and a P1-19 reconciliation item, the second verifying all four closed with 44 unit tests and 7/7 daemon cases in both modes; batch gate on main follows).

## P5-06

Receipt: [P5-06.md](P5-06.md) (branch ntm/fbittorrent/cod_2, commit cfda9b5; merged by the orchestrator after an Astra review that re-ran the three fork tests independently; the review's non-blocking P3, boxing the snapshot outside the chunk-tracker locks, is bead P5-06b; batch gate on main follows).

## P6-07

Receipt: [P6-07.md](P6-07.md) (branch ntm/fbittorrent/cod_4-p6-07-9bt, commit 73c22f9; merged by the orchestrator after an Astra review; the daemon.py conflict with P1-03's harness was resolved additively by the orchestrator; its 19-line docs/IPC.md note is folded into P1-05's document when that merges; the disclosed startup-teardown flake is bead P1-03b; batch gate on main follows).

## P5-06b

Receipt: [P5-06b.md](P5-06b.md) (branch ntm/fbittorrent/cod_2-p5-06b-vt1, commit 0c7ac31; the exact change prescribed by the P5-06 review, verified by the orchestrator from the diff; batch gate on main follows).

## P1-10

Receipt: [P1-10.md](P1-10.md) (branch ntm/fbittorrent/cod_2-p1-10-zxk, commit a6b520d; merged by the orchestrator after an Astra review that re-ran the adapter tests and the restart/shutdown daemon cases; batch gate on main follows).

## P1-16 (engine half)

Receipt: [P1-16.md](P1-16.md) (branch ntm/fbittorrent/cod_4-p1-16-vi1, commits a93b900 and f1e6fac; merged by the orchestrator after two Astra reviews, the first returning one P2 on dormant size/completed_bytes for partially selected torrents, the second verifying the selected-piece accounting against the live path; the daemon.py cases are bead P1-16b; batch gate on main follows).

## P1-16c

Receipt: [P1-16c.md](P1-16c.md) (branch ntm/fbittorrent/cod_4-p1-16c-yur, commit e7af809; merged by the orchestrator after an Astra review that re-ran the typed-seed tests; the queue-reconciliation defect it surfaced is bead P1-16d; batch gate on main follows).

## P1-16d

Receipt: [P1-16d.md](P1-16d.md) (branch ntm/fbittorrent/cod_4-p1-16d, commit b84e09b; merged by the orchestrator after an Astra review that re-ran the reconciliation regressions and the whole engine suite; batch gate on main follows).

## P1-05

Receipt: [P1-05.md](P1-05.md) (branch ntm/fbittorrent/cod_3-p1-05-2qa, commits 5052283, 573dc12, 7044f6f, b87e031 and 3a4caac, the last three applied by cod_3, cod_4 and cod_2 in turn; merged by the orchestrator after five Astra reviews that progressively enforced the bead's literal operation list, the app.info shape, complete fixture coverage, required pending refs, value validation, watch-gated playable fields and events, and the append-only events topic; batch gate on main follows).

## P1-03b

Receipt: [P1-03b.md](P1-03b.md) (branch ntm/fbittorrent/cod_3-p1-03b, commits abe3dad and 95291d4; merged by the orchestrator after two Astra reviews, the first returning two P2 findings (a 25 ms scheduling window in the regression, unfinished handles dropped on join timeout) that the second verified closed with 3/3 runs of each of the five startup tests; the historical cause stays open in P1-03c; batch gate on main follows).

## P1-04

Receipt: [P1-04.md](P1-04.md) (branch ntm/fbittorrent/cod_1, commits 5a2fba2, f8bf0ab and 3061ee0 with the orchestrator's reconciliation merge eba7d2c; merged by the orchestrator after three Astra reviews. The receipt's incident section records that during development five disposable units started the daemon with the manager's default environment (22:16, 2026-10-18); the orchestrator's assessment: no file under ~/.local/share/fbittorrent changed, the installed binary and unit directories are untouched, seven new payload files appeared under ~/Downloads for the owner's one live torrent (two empty, three sparse shells, two with a few hundred KiB downloaded) and the pre-existing E08 file verifies 710/710 fully contained pieces against the torrent hashes; nothing was deleted; batch gate on main follows).

## P1-02b

Receipt: [P1-02b.md](P1-02b.md) (branch ntm/fbittorrent/cod_3-p1-02b, commit efe36ca; test-only fix of a pre-existing fixture readiness race in the transfer tests, merged by the orchestrator after an Astra review that recomputed the retained timelines and re-ran the transfer tests; batch gate on main follows).

## P1-03c

Receipt: [P1-03c.md](P1-03c.md) (branch ntm/fbittorrent/cod_3-p1-03c, commits 64bd004 by cod_3 and 34b9cee, fdd1298 by cod_4; merged by the orchestrator after three Astra reviews, the first two returning one P2 each (unbounded diagnostic collection on the shutdown thread; short socket writes truncating the record) and the third verifying both closed with 36/36 independent runs; includes an additive try-lock snapshot API in the vendored Asupersync inspector recorded in vendor/asupersync/FBITTORRENT_PATCH.md; batch gate on main follows).

## P1-05b

Receipt: [P1-05b.md](P1-05b.md) (branch ntm/fbittorrent/cod_3-p1-05b, commits a3109de, 6a7f58e and dfac645; fixture corrections surfaced by P1-06's live replay, merged by the orchestrator after an Astra review with no findings; batch gate on main follows).

## P1-05c

Receipt: [P1-05c.md](P1-05c.md) (branch ntm/fbittorrent/cod_3-p1-05c, commits 7c07cdf and 84c17db with the orchestrator's merge a8c6dec; fixture corrections surfaced by P1-06's live replay, merged by the orchestrator after an Astra review with no findings; batch gate on main follows).

## P1-05d

Receipt: [P1-05d.md](P1-05d.md) (branch ntm/fbittorrent/cod_3-p1-05d, commit 28c64da; fixture correction surfaced by P1-06's live replay, merged by the orchestrator after an Astra review with no findings; fixture-only change, covered by the next batched gate on main).
