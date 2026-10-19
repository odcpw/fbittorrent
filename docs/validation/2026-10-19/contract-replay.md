# M1a gate9: torrent catalogue and real transfer replay

Base: `eb3eda0` (includes the workflow_dispatch-only CI change).

The ETA implementation did not change: its current lines originate in P1-19
`9e60087`. Start classification originates in P1-06 `33bc9f4`; the later
`eb72fa6` changed error wording, not completed-versus-running classification.
P5-05 `c155727`, integrated in `50188d2`, advertised `watch` and therefore
activated catalogue frames that the replay had previously deferred.

Gate9's line-13 live row was stalled (0 B/s), so it correctly omitted ETA. Its
4 KiB payload then completed before the first detail rate sample; Start correctly
returned invalid_state / complete_no_seed. The catalogue also combines globally
parked and running rows and claims playable=true for a 1 KiB prefix, contrary to
PLAN DL-26's minimum 32 MiB. The live validator separately treated completion as
constant across receiving samples. These are replay assumptions, not reasons to
fabricate ETA or change the engine's completion policy.

The catalogue remains byte-identical to eb3eda0; its strict full-group round trip
and field coverage remain intact. `contract_torrents` validates that catalogue,
then validates every original captured frame from reproducible real scenarios.
An 8 MiB two-file payload with a 32 KiB/s native seed proves incomplete receiving,
list/detail ETA arithmetic and the exact already_running start result. A separate
completed payload proves complete_no_seed rejection. All ten states are observed
in real transitions, including separate VPN epochs and missing-file activation.
Mutation responses, file geometry/progress, duplicate add, keep-files/undo, file
selection, seed/upload policy, open_target, shutdown reference and bye are checked.
No runtime engine code, protocol types, capabilities or version changed.

The live validator permits advancing completion while receiving, but preserves
raw tuple checks, static file geometry, strict catalogue checks and revision
validation. Regression controls reject backwards progress, paused drift, wrong
progress, impossible completion and changed size. Existing checking/removal
controls remain unchanged and pass.

Validation in integration:
- Wrapped rch `cargo build --release --locked`: pass; shared release lock and
  MemoryHigh=10G / MemoryMax=14G. Receipt: batch17-contract-replay/release-build.log.
- Conditional ETA unit: 1 pass (the initial exact filter selected zero tests;
  corrected full name is recorded in eta-unit-selected.log, with no credit for
  the zero-test invocation).
- `cargo check --locked --tests`, Python compilation, case discovery and size
  lint: pass. Six focused live-validator tests plus strict fixture roundtrip:
  7 passed, no ignores. Logs: batch17-contract-fix/.
- Release daemon.py --no-systemd --contract-validator target/batch17-contract-validator
  --case contract_torrents --case contract_hello --case contract_errors --jobs 1:
  159/159, 24/24 and 88/88 steps respectively, exit 0. Original catalogue and
  captured live trace both pass the frozen Rust validator. Full logs and SHA-256
  catalogue equality evidence: batch17-contract-live-fixed/.

All log roots above are under target/test-logs/integration/. The first new-driver
run retained a missing app.shutdown args failure; it was corrected, then all
three requested cases reran. No full suite, all.sh, hosted Actions run or new
feature merge was performed. The orchestrator owns the integrated green gate.
