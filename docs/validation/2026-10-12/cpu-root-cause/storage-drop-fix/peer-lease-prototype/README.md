# Peer storage lease implementation and scoped validation

The root agent authorized integration after the qualified actor pilot. The live change is limited to three production surfaces: `spawn_utils.rs`, `spawn_utils/storage_owner.rs`, and four hunks in `torrent_state/live/mod.rs`. Saved corrected actor and minimal per-operation candidates remain unchanged. This directory retains the final helper/test copies, production/test patches, pre-EOF module, source manifest and executed receipts.

## Changes

- `StorageLease` belongs to each incoming/outgoing peer handler. Construction captures the original eligible `RegionOwnerKey` (including priority) and an empty `OnceLock`; idle peers allocate no workers or cache entries. The first actual operation checks the actual caller key, uses existing cache arbitration, rechecks authority after the await, then stores a strong Group without another await. Generic `BlockingSpawner::run` uses the existing per-operation helper outside an explicit lease.
- Cache entries contain weak Group and Shared references. Last handler sender drop closes the inbox naturally. A draining key remains reserved while its Shared has live workers, and the separate 16-group slot is released only when the last worker physically exits. A full/draining cache uses the safe per-operation fallback; None is never cached, so the same lease can recover later. An accepted callback is never retried.
- Workers now distinguish genuine sender EOF from cancellation/error. An EOF worker disarms its fail-closed guard; it does not cancel siblings or discard their already accepted work. Cancellation, error, unwind and pre-first-poll denial still cancel the group and close/drain its inbox. Both outcomes release their slot only on the last worker exit, after physical drain.
- A common `Shared::retire_worker` now handles actual worker exit and abandoned spawn reservation alike. A last 1-to-0 rollback releases quota even if a stopped Group remains retained. The exact race-window regression and OLD/NEW receipts are documented in `retirement-rollback/README.md`.
- `PoolWork::drop` catches a single callback capture destructor panic, destroys physical captures before terminal publication, and sends its original panic payload to the uninterruptible completion receiver. Normal pool rejection retains its error path. No scheduler-inline recovery or new thread fallback was added when a pool exists.
- The original ten owner tests use explicit leases, with all 71 assertion token sequences and original two-second/50 ms deadlines preserved. A new included file adds nine tests. Upload remains on its existing helper and common admission semaphore.

## Executed evidence

Rust 1.98, `--offline --locked`, `CARGO_BUILD_JOBS=2`, no RUSTFLAGS, one cargo operation at a time, protected repository target. The persistent runner records PID, command, source hashes, exit status, log hash and final source restoration. No release build, benchmark, real transfer or user-state operation was performed by this agent.

| Run | Result | Evidence |
| --- | --- | --- |
| Saved pre-EOF module, final unchanged EOF regression, explicit `--lib` | Expected RED: 0 passed / 1 failed, exit 101 | `old-eof-receipt.json`, `old-eof-tests.log` |
| Original rollback plus identical test-only scheduling hook, final retirement regression | Expected RED: 0 passed / 1 failed, exit 101 | `old-retirement-receipt.json`, `old-retirement-tests.log` |
| Final live owner suite, explicit `--lib` | GREEN: 19 passed / 0 failed, exit 0 | `lease-receipt.json`, `lease-tests.log` |
| Unchanged actual application ownership tests, `--bin fbittorrent` | GREEN: 2 passed / 0 failed, exit 0 | `native-receipt.json`, `native-tests.log` |

The EOF regression uses two accepted jobs, a real prefix-held file write and a native control owner holding global admission ahead of the second job. On OLD, genuine sender EOF sets stopping=true and the second callback never executes. On NEW, the idle worker exits while the accepted sibling remains alive: one slot is held, zero permits are available, one admission waiter remains, and the second callback is still pending for 50 ms. After explicit release it writes its complete bytes exactly once. Both runs release physical barriers and the control owner's permits, join that owner, and complete fixture close/shutdown before their semantic assertions.

The first eight new tests cover 33 successive actual peer task contexts; draining same-key duplicate prevention and the 16-slot bound; peer departure with a held real write and natural EOF drain; held region close with sibling survival; 33 idle eligible contexts with zero owners; saturated first-call fallback followed by recovery and reuse of the same lease; graceful EOF with an accepted sibling waiting admission; and actual queued native-pool discard with a pending native child and panicking callback captures. The discard test confirms original typed panic wins simultaneous cancellation, F executes zero times, and admission is restored before terminal join.

The ninth test controls only a post-CAS scheduling pause, uses actual native worker exit/receiver loss and the actual rollback branch, retains the retired Group, and proves the full quota can recover for a real cached operation. Both original retirement and corrected behavior complete cleanup before assertions. Prior 18-test and pool2 app2 receipts remain under `pre-retirement-*`; current app2 captures root's experimental max1 NativeRuntime source.

The prior queued capture-panic regression also remains GREEN: while another real file callback is held, queued destruction reports typed Panicked, admission stays at zero, an owner stays alive, and region close remains Pending for 50 ms until release. The separate reject-path owner-survival test passes. `assertion-preservation.json` records original assertion/deadline preservation; application test source remains SHA `f2227a2e22cc24d51416a56dd20da2bd35b4bf7d946486f96b81ec905a0d45d2`.

The first attempt stopped at compilation because the peer constructors needed rqbit's existing `Error::Anyhow` mapping; that log/receipt is preserved as `compile-attempt-*`. The next semantic attempt, before explicit target scoping and robust EOF observation, is retained as `first-semantic-*`. These are separate from the final accepted receipts.

## Source review and limits

Physical permit, owner ambient guard, callback captures and job state are destroyed before terminal reply; accepted physical futures are retained and drained uninterruptibly on cancellation. The original pool, capability, scope, key/priority and budget eligibility remain authoritative. Actual caller mismatch or ineligible original identity uses per-operation fallback. Existing denied-IO/spawn tests retain the host free-filesystem boundary; no filesystem capability policy changed.

An ineligible original context remains per-operation for that lease. A stored Group that later stops remains in the OnceLock, so that lease subsequently falls back rather than replacing the stopped Group. The patch adds no fairness policy beyond the existing bound, no idle timers, and no universal protection against two destructors panicking during the same unwind. Scoped source tests establish ownership/lifecycle behavior; the earlier pilot does not measure this lifetime change. Normal release and frame-pointer builds, actual peer transfer acceptance and performance qualification remain root-owned.
