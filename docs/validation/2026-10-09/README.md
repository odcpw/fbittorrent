# Native engine validation — 2026-10-09

This index records the rqbit 9.0.1 port to Asupersync 0.5.0, the supported
fbittorrent client behavior, and the evidence collected on this host. The
[engine evaluation](../../engine-evaluation.md) describes the implementation,
ownership model, supported controls and capability limits. The
[fork notes](../../../vendor/rqbit/README.md) record upstream provenance and the
maintained native surface.

## Final receipt status

| Item | Final receipt |
| --- | --- |
| Native port and focused storage/network checks | Completed for the used client surface. |
| App/runtime tests after the final Session caller checkpoint correction | [Clean final rerun](app-runtime-tests.txt): 62 app and 6 runtime tests passed. |
| Installed binary identity | [Installation receipt](installation.json): 16,813,160 bytes; optimized `release-perf` build with debug symbols stripped. |
| Resource measurement of the final installed, stripped binary | [Final receipt](resources.json) measures the exact installed SHA256 `aeb1f312ff0a92a2d99089f1ed017112fefd97f5ff429df7b46966c56c1fc1c9`. |
| Registered `.torrent` file URI through `xdg-open`, with literal plus, spaces and Unicode, from an app that is closed | [Passed](desktop-integration.json); real app opened and existing local fixture restored. Dispatch observation: 1033.123 ms. |
| Registered duplicate magnet through `xdg-open`, from an app that is closed | [Passed](desktop-integration.json); real app opened and existing local fixture restored. Dispatch observation: 364.515 ms. |
| Live widget tooltip | Fixed position, theme colors and wrapped full text checked; [screenshot saved](widget-tooltip.png). |
| Live Shows source | [Passed in isolated state](shows-live.json): the installed app followed a test IMDb title and discovered 38 episode/season entries from EZTV, without starting its engine or downloading payload. |

The installation and final resource receipts have the same binary SHA256. The
earlier `c4fc2eae…` measurement is preserved in `resources-initial.json`.

## Evidence artifacts

| Artifact | What it records | Scope |
| --- | --- | --- |
| [Engine evaluation](../../engine-evaluation.md) | Engine choice, native regions/tasks, durable state, cancellation, controls and reproducible checks. | Implementation and checked client surface. |
| [Normal dependency graph](normal-dependencies.txt) | The application's normal dependency tree, including native Asupersync and the vendored rqbit family. | No Tokio, reqwest or hyper in this saved normal graph. Test-only dependencies and archived upstream references have a separate scope. |
| [Native integration output](native-integration.txt) | Closed status, local RSS, one Enter adding once, read history, CLI fields, file selection, upload cap, deletion confirmation, file modal, Shows/help, quit/restart and app close. | Rerun passed against the final installed binary, in isolated state with local fixtures. |
| [Final app/runtime tests](app-runtime-tests.txt) | All 62 app and 6 runtime test results after the final caller checkpoint correction. | Clean exit and captured background-runtime output. |
| [Desktop integration receipt](desktop-integration.json) | Registered file URI and duplicate magnet, actual terminal launch, fixture restoration/removal, final app close, legacy service state and migration counts. | Real installed desktop entry paths with a locally generated stopped torrent; no public download. |
| [Final closed state](closed-state.json) | Matching installed hash, no engine process/socket, inactive/disabled legacy service, and removal of the temporary fixture directory. | Read-only final host verification after all integration and resource runs. |
| [Live Shows receipt](shows-live.json) | Local follow, live EZTV check, and cached episode/season keys through the installed CLI. | Temporary watchlist removed afterwards; no user watchlist or downloads changed. |
| [Host fingerprint](fingerprint.json) | Omarchy/Linux host, CPU, memory, Btrfs, Rust 1.98.0, build profile and observed kernel/power settings. | Same-host context. The source checkout was dirty; the Git revision alone does not identify all tested changes. |
| [Source identity](source.json) | Aggregate SHA256 over 173 Cargo/lock, application/vendor Rust and widget QML/JavaScript files, using sorted paths and bytes separated by NUL. | Explicit source scope; generated target files, documentation and the rest of the checkout are outside this digest. |
| [Initial resource baseline](resources-initial.json) | Raw startup, status, shutdown and idle process samples for the earlier `c4fc2eae…` binary. | Preserved historical baseline; startup label names the witnessed engine socket. |
| [Resource receipt](resources.json) | Raw distributions, binary hash, RSS/PSS/peak RSS, thread waits and idle CPU. | Final installed, stripped binary; hash matches the installation receipt. |
| [Installation receipt](installation.json) | Installed path, binary hash, byte size, optimization profile and backup directory. | `/home/oliver/.local/bin/fbittorrent`, with the previous installation backed up. |
| [Live widget tooltip screenshot](widget-tooltip.png) | The themed, wrapped full-name tooltip at its corrected popup position. | Layout/full-text evidence, captured before the final seeding-label wording correction. |

## Test coverage

The collected suite runs account for **168 passing tests**, with one additional
Miri-only helper ignored. The final app/runtime rerun passed after the Session
caller checkpoint correction; its [captured log](app-runtime-tests.txt) contains
no worker panic or obligation leak.

| Suite | Passing tests in collected runs | Representative evidence |
| --- | ---: | --- |
| App | 62 | Local magnet/payload transfer, pause/close/reopen, persisted upload policy, corrupt records, mismatched metadata before file creation, failed deletion persistence, pending HTTP cancellation through IPC, search provider children, RSS/Shows and terminal controls. |
| Native runtime facade | 6 | Real native context, scoped ownership, cancellation and runtime drain behavior. |
| DHT | 28 | Native wire fixtures, discovery state, cancellation and timer behavior. |
| Dualstack sockets | 4 | Actual sockets, canceled UDP receive and port release. |
| Tracker communications | 5 | HTTP/UDP tracker wire fixtures and cancellation. |
| Storage/library | 31 | Positioned vectored writes, persisted bitfield flush/drop, exact/partial/multiple-file hashes, symlink sentinels, and a file stream returned by one task and read by another. One Miri-only helper is ignored. |
| Core | 32 | Piece/chunk algorithms, malformed sizes and hash tables, metainfo/hash serialization, and parked-task cancellation with a cleanup witness. |
| **Total** | **168** | Collected native suites, including the clean final app/runtime rerun. |

The [main regression runner](../../../agent_ergonomics_audit/audit/regression_tests/all.sh)
checks captured output for background worker panics and obligation leaks as well
as the process exit status. A caller-thread success cannot hide a native worker
failure. Focused storage/core runs also used `--nocapture` and completed without
worker panic or obligation leak output.

## Migration and desktop observations

The live legacy migration completed with **0 torrents, 1 feed and 18 providers**.
The old qBittorrent service is inactive and disabled, and its backup was kept.
The [desktop receipt](desktop-integration.json) records those final states.
This establishes the actual local migration result; downloaded-payload
preservation and torrent policy restoration are additionally exercised by the
isolated torrent fixtures.

Real integration exposed three narrow entry-path defects:

- A launched terminal inherited the invoking CLI's stderr pipe, keeping the
  caller waiting for EOF. The launcher now detaches its standard streams.
- Generic `xdg-open` ignored a quoted executable in the desktop entry. Safe
  ordinary executable paths are now emitted unquoted; paths requiring desktop
  escaping retain that handling.
- Adding an already managed magnet fetched metadata peers again. The native
  add route now returns the existing handle early when `list_only` is false,
  after checking the caller's cancellation context.

The final registered file-URI and duplicate magnet checks passed from an app
that was closed. Both opened the actual native app and restored the generated
local fixture. The fixture and its payload were removed afterwards, the app
closed, and the legacy service remained inactive and disabled. The two dispatch
durations are individual observations, not latency distributions.

## Resource baseline and final measurement

The preserved initial baseline used an optimized, symbol-preserving
`release-perf` binary with SHA256
`c4fc2eae0055ed78882656cb5063820b35cbedd5d67110707ecf7c7ce9df4407`.
The [raw initial receipt](resources-initial.json) supplies the baseline column
below. The [final receipt](resources.json) measures the installed, stripped
binary with SHA256
`aeb1f312ff0a92a2d99089f1ed017112fefd97f5ff429df7b46966c56c1fc1c9`.
These are observed same-host receipts. They do not attribute a performance
change to a particular fix or establish comparative engine performance.

| Measurement | Initial saved baseline | Final installed receipt |
| --- | --- | --- |
| Launch to engine socket, 20 samples | p50 16.29 ms; p95 21.02 ms; p99/max 22.79 ms | p50 8.64 ms; p95 10.93 ms; p99/max 10.97 ms |
| Status CLI, including CLI process startup, 39 samples | p50 2.22 ms; p95 7.73 ms; p99/max 8.00 ms | p50 2.36 ms; p95 5.06 ms; p99/max 6.92 ms |
| Quit to process exit, 20 samples | p50 1416.69 ms; p95 1516.91 ms; p99/max 1617.06 ms | p50 1516.41 ms; p95 1667.12 ms; p99/max 1719.47 ms |
| Idle RSS after the measured window | 34,832 KiB | 35,524 KiB (34.69 MiB) |
| Idle proportional set size after the measured window | 31,344 KiB | 32,022 KiB (31.27 MiB) |
| Process peak RSS | 85,324 KiB | 84,496 KiB (82.52 MiB) |
| Threads after the idle window | 8 | 8 |
| Idle CPU over approximately 10 seconds | 0.400% of one CPU core | 0.300% of one CPU core |

The scenario is an empty native TUI at 120×30, fresh isolated config/data/cache/
runtime directories, and normal discovery. The executable cache is warm. The
startup endpoint is engine socket creation; it does not measure the first
visible TUI frame. Shutdown begins with the terminal's quit key and ends with
process exit and socket removal. Status samples include launching the separate
CLI process. Memory values are KiB from procfs, and idle CPU uses the process's
user/system tick delta. The idle window is one final-run sample, not a memory
distribution. With fewer than 100 samples, the reported p99 is the maximum.

## Evidence and review decisions

The practical JSM Rust port, Asupersync, profiling and optimization review was
bounded to the client's used behavior. It combined independent fresh source
inspection, real integration, native ownership/cancellation checks, fault cases
and a measured baseline. Full ten-round gauntlet certification is outside this
record.

| Observed issue or measurement | Narrow action or decision | Evidence and remaining bound |
| --- | --- | --- |
| The engine's normal effects still depended on upstream runtime surfaces. | Port the used engine, DHT, trackers, sockets, HTTP, storage and task/context boundaries to native Asupersync. | Saved normal graph and native suites; optional archived upstream surfaces are outside the app build. |
| A peer or stream permit could outlive the task that acquired its native obligation. | Acquire peer permits in peer tasks; acquire and release stream permits per bounded read in the reading task. | Native ownership checks and the real stream crossing a completed creator task. |
| Cancellation could settle a blocking join before the actual disk closure finished. | Observe physical completion before restarting initialization or finishing torrent close. | Real storage/session cancellation checks; disk work checks cancellation between pieces. |
| Short positioned writes and hash boundaries could corrupt disk results or metainfo. | Complete partial vectored writes; correct exact/partial/multiple-file hashing; reject invalid length/hash ranges. | Real disk byte comparisons and core boundary tests. |
| Invalid saved records, wrong cached metadata or descendant symlinks could produce unsafe file effects. | Preserve invalid ledgers, validate hashes before disk admission, and anchor Unix relative open/delete operations without following descendant symlinks. | Corrupt/mismatch checks and preserved outside-file sentinels. |
| A failed record save could allow deletion to resurrect a torrent on reopening. | Save record removal before deleting payload; exercise a forced save failure and reopen. | Final app fault checks passed. |
| Selected search results sometimes contain detail URLs or opaque provider IDs. | Run only the selected provider's bounded download hook, then add its resolved magnet/URL or validated torrent bytes. | Real Python hook/file fixtures, subprocess cleanup, direct-link bypass and manual-add behavior. |
| Launcher pipe inheritance, desktop executable quoting and duplicate magnet lookup failed under real entry paths. | Apply the three narrow entry-path fixes described above. | Final registered file-URI and duplicate magnet checks passed. |
| Empty-app shutdown is the largest measured latency: approximately 1.42 seconds median initially and 1.52 seconds in the final run. | Preserve the measured close bound and its raw samples; keep further optimization tied to a demonstrated cause. | Final installed-binary receipt saved; no attributed speedup or comparative engine claim. |
| Idle process memory and CPU were measured; heap allocation and stack costs were not sampled. | Report procfs bounds and preserve the measurement limits. | Initial and final RSS/PSS/HWM and tick samples above; allocation profiling remains unmeasured. |

## Limits of the evidence

- Payload transfer is a real local **256 KiB** fixture with wire-level tracker,
  DHT and socket fixtures. Long WAN soak, private tracker interoperability and
  sustained swarm throughput remain unmeasured.
- The resource scenario has an empty library. Reopening existing payload hashes
  disk data, so large-library startup cost is outside these measurements.
- Procfs RSS/PSS/peak RSS describe process memory. Heap allocation peaks,
  per-operation allocation counts and stack profiles were not measured.
- Comparative throughput or memory against IronTide or other engines was not
  measured. The documented engine choice is based on integration fit.
- The supported controls and unavailable protocol/service features are listed
  in the [engine evaluation](../../engine-evaluation.md).
- The resource receipt covers the installed binary identified above. Its
  fingerprint and the source digest define the checked local build context.
