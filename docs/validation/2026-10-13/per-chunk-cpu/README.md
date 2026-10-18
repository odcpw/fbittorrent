# Receive and storage CPU results

The adaptive receive pipeline used **about 23–24% less CPU** than the preserved native baseline in two completed twenty-pair campaigns. Median peak PSS remained **about 53% below same-TUI qBittorrent**, native wire throughput stayed at the **32 MiB/s seed cap**, and app-lifetime engine shutdown was preserved in the tested runs. Both campaigns, independent reviews and the ten-round scoped gate passed. The exact measured normal executable was installed on 14 October 2026. This directory retains 13 October because work began then.

The measured candidate is source `cf0fbcfb…`, normal executable `5ede27d8…`; baseline is `d72971dc…`. The [build receipt](/home/oliver/Projects/odcpw/fbittorrent/docs/validation/2026-10-13/per-chunk-cpu/coalescing-adaptive-v1-build.json) binds Rust 1.98, release-perf, empty explicit features and no extra Rust flags to unchanged source. Asupersync and FrankenSuite remain in use.

## Completed final measurements

| Result | [R9](/home/oliver/Projects/odcpw/fbittorrent/docs/validation/2026-10-13/per-chunk-cpu/round_9/perf-summary.json) | [R10](/home/oliver/Projects/odcpw/fbittorrent/docs/validation/2026-10-13/per-chunk-cpu/round_10/perf-summary.json) |
| --- | ---: | ---: |
| Actual matched blocks | 20 | 20 |
| Baseline / candidate / qBT median CPU | 5.850 / 4.460 / 1.390 s | 5.835 / 4.430 / 1.400 s |
| Median paired CPU reduction | **23.30%** | **24.40%** |
| Lower-CPU / higher-CPU blocks | 19 / 1 | 19 / 1 |
| Paired bootstrap 95% interval | 21.89–24.26% | 22.45–25.38% |
| Baseline / candidate / qBT median peak PSS | 44.202 / 35.053 / 74.539 MiB | 42.147 / 34.114 / 73.584 MiB |
| Candidate PSS below qBT | 52.97% | 53.64% |
| Baseline / candidate median completion | 16.106 / 16.116 s | 16.102 / 16.105 s |

Native wire medians remain approximately 15.9996 s in both variants and rounds. Candidate PSS is 9.148/8.033 MiB below the native baseline. Opening, status, close, total-stop and reopen medians all meet their original allowances; the small R10 opening increase is 2.293 ms. The [combined numerical guards](/home/oliver/Projects/odcpw/fbittorrent/docs/validation/2026-10-13/per-chunk-cpu/round_10/composite-numerical-guards-v3.json) pass the twenty-pair, ≥16 lower-block, >3% median, positive interval, RAM, completion, wire and response requirements.

Candidate median CPU is still **3.21×/3.16× qBT** in these same 512 MiB tests. The earlier approximately 4.7× figure came from a different scenario and is not a direct before/after comparison. The common seed cap does not mean qBT achieved the same wire rate.

## Why the change helps

Previously each 16 KiB Piece payload was cloned and submitted separately. The receive allocation now travels through existing storage ownership and returns after successful processing. One physical callback processes up to four consecutive frames serially, retaining original caller/key and physical-owner checks, checksum/event/error order, and control/transport fences.

Feature-only [high-rate diagnostics](/home/oliver/Projects/odcpw/fbittorrent/docs/validation/2026-10-13/per-chunk-cpu/adaptive-high-rate-count/profile.json) recorded 32,768 frames but 8,313 handoffs: **74.63% fewer**, mean depth 3.942, zero explicit ownership-payload clone bytes. The [fresh FP review](/home/oliver/Projects/odcpw/fbittorrent/docs/validation/2026-10-13/per-chunk-cpu/adaptive-post-change-fp/attribution-reviewed.json) recovered 1,251 user-CPU samples with zero reported loss/unparsed lines; actual callback/poll anchors occur in 326/85 samples. Storage handoff outside the callback represents 6.71% versus baseline 11.14%; SHA1 leads physical mechanisms at 23.26%. These shares are compositional; inclusive envelopes overlap and contain task bodies. Counters/profile builds are separate from normal CPU measurements, exclude kernel/suspended async-chain attribution, and do not measure isolated handoff cost.

The original unsuccessful stages remain visible: copy-only R1 saved 0.25%; passive R2/R3 returned −0.49%/−2.25%. Unconditional waiting regressed the slow probe by **33.89%**. The current N8 countdown skips timed intake for eight passive batches after a singleton timed attempt and periodically probes for recovery. [Slow-rate evidence](/home/oliver/.codex/artifacts/fbittorrent-chunk-20261013/adaptive-backoff-causal-review-facts.json) shows **88.88% fewer waits**, 455 versus 4,093. The normal 64 MiB/4 MiB/s pilot is near-neutral: 0.38% median reduction, two wins/two losses; it passes its predeclared 5% regression allowance without establishing a CPU win.

## Workload, ownership and evidence limits

Both final campaigns use the same 120×30 TUI, private **512 MiB loopback TCP** payloads, fresh destinations on **/tmp tmpfs**, warm executable/payload caches and actual shared ratings data with SHA `bb71c804…`. Each accepted segment has fresh two×30-second host qualification, fixed-worker checks, continuous sampled monitoring, before/after fingerprints and actual fifteen-transfer cache observations.

Each round comprises five four-block segments, with three retained warmups each: **30 warmup transfers across both rounds**, 24 more than one process per round. Global orders and idle positions are preserved. These are five clusters per round, with fresh process/seed/cache lifetimes and time gaps. Whole failed attempts and noisy/interleaved preliminary campaigns remain archived and excluded; healthy segments, including their negative CPU blocks, are retained. No partial timing selection is used.

The paired intervals are conditional empirical intervals, not proof of independence or portability. Supplemental cluster bootstrap ranges are **21.45–24.23%** and **22.99–25.00%**; only five observed clusters make these sensitivity checks, not stronger certification or replacement gates. Sampled host/cache checks cannot exclude every transient job or replacement between observations.

The [R9 facts](/home/oliver/Projects/odcpw/fbittorrent/docs/validation/2026-10-13/per-chunk-cpu/round_9/composite-review-facts.json) and [R10 facts](/home/oliver/Projects/odcpw/fbittorrent/docs/validation/2026-10-13/per-chunk-cpu/round_10/composite-review-facts.json) retain **150 exact full payload hashes/seed joins**, **100 native zero-payload reopens**, and **200 native close/reclose zero-resource observations**, counting baseline and candidate together. Each round also passed a fresh three-download partial close/resume case with exact final hashes. qBT backend-after-TUI-close metrics are not zero-resource claims.

Fresh conformance includes 27 ownership, two app, 16 actual provider cases, 24 parser, 23 protocol and one injected Live fatal-write test. The warmed buffer remains 65,588 bytes, 32,820 above initial; bounds remain four frames/Ready reads, one buffer/handoff per peer, cache 16/admission 2 and one scheduler/max two blocking workers. Timed attempts share one absolute 1.5 ms cutoff; observed maximum wait 3.092 ms shows scheduler overshoot. Many-peer RAM, Internet, cold disk, VPN, battery, maximum throughput and extreme tails are unqualified. Miri remains ignored; full Live retirement/reconnect, real OS disk faults and isolated cancellation wake latency are unclaimed. Delayed control/EOF fixtures use a private 50 ms cutoff for transition logic. This is scoped engine validation, not a full release certificate.

## Acceptance and installation

The [ten-round validation](final-gauntlet-validation-v2.json) passed with both final rounds clean on identical source and executable, all hypotheses resolved, and no open genuine finding. Independent [R9 raw review](/home/oliver/.codex/artifacts/fbittorrent-chunk-20261013/round_9-final20-composite-independent-raw-review-v1.json) and [R10 raw review](/home/oliver/.codex/artifacts/fbittorrent-chunk-20261013/round_10-final20-composite-independent-raw-review-v1.json) bind the actual receipts. A stale R1 ledger reference was [repaired](hypothesis-copy-only-reference-repair.json) through its preserved original; failed validation and all original bytes remain retained. No metric or gate was weakened.

The [root decision](final-root-keep-decision.json) kept the candidate. [Atomic installation and final state](installation-and-after-state.json) confirm `/home/oliver/.local/bin/fbittorrent` SHA256 `5ede27d852dc0568d0fb3330a069aede13ff721cb532e30035945b549f148ade`, exactly the normal executable tested. The preserved `baseline-installed` artifact remains available for rollback.

The app remains closed and its native engine stopped; installation did not start or restart it. All eight guarded files match the recorded post-edit guard. An earlier unrelated desktop-shell edit was preserved. Torrent/Shows/RSS/search/settings data and the qBittorrent unit remain unchanged; that unit is inactive, dead and disabled. Independent lifecycle reviews establish fixture cleanup and recorded owned-process exit. The optimization is ready on the next app launch.

The [independent final gate audit](/home/oliver/.codex/artifacts/fbittorrent-chunk-20261013/final-scoped-gate-reference-repair-independent-review-v1.json) also verified the ledger repair and all ten completed rounds without rerunning tests or changing thresholds.

## Subsequent launcher correction — 14 October 2026

The app launcher and widget were corrected after these measurements because a bare window pattern focused an unrelated project terminal. The [launcher report](../../2026-10-14/launcher-fix/README.md) records the new installed executable and actual desktop/widget verification. Engine and runtime sources remain byte-identical to the measured build; launcher/setup code changed, and the new executable was not separately benchmarked. The original benchmark artifacts, installation and completion receipts above remain historical evidence.
