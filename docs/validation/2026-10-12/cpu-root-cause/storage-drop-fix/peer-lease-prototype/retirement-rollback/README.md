# Last spawn-reservation retirement correction

Authorized live follow-up to the lease review. `Shared::retire_worker` owns the existing last-1-to-0 slot release. Both actual worker exit and receiver-upgrade failure after a successful spawn reservation call it. Physical worker exit remains after physical drain; the rollback path starts no physical worker. Cache retirement and cancellation/error policy are unchanged.

The regression controls scheduling with a per-Shared, test-only oneshot pause immediately after the **actual** Group::run worker-reservation CAS. No callback execution or owner count is fabricated. Sixteen groups are warmed with actual operations. One group runs a real prefix-held file write; a second run reserves expansion and pauses before receiver upgrade. Region close stays Pending for 50 ms while the write is held. After release, the file completes and the actual last native worker exits, leaving only the outstanding reservation and no receiver. The paused run resumes the real failed-upgrade rollback while its lease/Group remain strongly retained. Its callback is never accepted/executed. An actual new operation then checks cached quota recovery and writes its own file.

The old module is the saved pre-correction module with the **identical cfg(test) scheduling hook only**; production retirement/rollback are original. The hook is absent from normal release builds. OLD and NEW use exactly the same regression. Both complete held-write release, native worker close, all remaining region closes and native fixture shutdown before assertions.

| Evidence | Result |
| --- | --- |
| `../old-retirement-receipt.json`, `../old-retirement-tests.log` | Expected RED: 0 passed/1 failed, exit 101; real reservation 2, before rollback `(alive=1, receiver_gone=true, slots=16)`, after `(0, slot_released=false, 16)`; replacement fallback, retained Group, rejected callback executions 0 |
| `../lease-receipt.json`, `../lease-tests.log` | GREEN: 19 passed/0 failed, exit 0; after `(0, slot_released=true, 15)`; actual replacement cached with 16 slots while retired Group still retained |
| `../native-receipt.json`, `../native-tests.log` | GREEN: unchanged application ownership tests 2 passed/0 failed, exit 0; root-owned experimental max1 NativeRuntime source captured unchanged |

Rust 1.98, explicit `--lib`/`--bin fbittorrent`, offline/locked/jobs2, protected repository target, no RUSTFLAGS, one cargo operation at a time. Persistent receipts contain command, source hashes, restoration, exit and log hashes. No release build, benchmark, transfer, user-state or NativeRuntime source edit by this agent.

This is a deterministic test of the real race window and rollback path, rather than statistical stress coverage of arbitrary schedules. Original ten owner assertions/deadlines are unchanged. The qualified helper source is recorded in the adjacent integration manifest; root owns updated pool2/pool1 production builds and comparison.
