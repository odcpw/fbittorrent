# Local client comparison, 2026-10-20

Native source `a9e455b`, binary SHA-256 `8a5544c260bd91a8e1246fd896b3eda3a40e0cca9c864fc7a6f57829d31890de`; qBittorrent 5.2.4, libtorrent 2.1.2.0, binary SHA-256 `39552701e4f06d690f0e46746fc98847fe621f6d1e0ea623e48e63d56e96e2d5`. Native uses the shipping optimization settings with line tables retained (`release-perf`).

Private HOME/XDG and payloads on tmpfs. Identical deterministic torrents, plaintext TCP, sequential and first/last enabled, 1/4/16 localhost seed endpoints. Three alternating client-order repeats. Every payload independently SHA-1/SHA-256 verified; all 54 primary trials qualified with zero payload upload, no emergency cleanup and no survivors. qB has a 1 KiB/s upload cap, with LAN protocol traffic exempt during controlled local trials; the wire counters prove zero payload upload.

Completion is observed through each API. Values are median [minimum–maximum] seconds. This shared-host run does not establish cold SSD performance.

| Payload | Seeds | Native completion | qB completion | CPU seconds native / qB | Peak RSS MiB native / qB | Positioned writes native / qB |
|---|---:|---:|---:|---:|---:|---:|
| 1 GiB / 1 files | 1 | 1.97 [1.89–4.48] | 1.50 [1.40–2.80] | 2.26 / 1.48 | 26.1 / 76.9 | 65,536 / 4,164 |
| 1 GiB / 1 files | 4 | 1.24 [1.15–2.43] | 2.82 [1.42–2.84] | 2.51 / 2.25 | 27.2 / 102.3 | 65,536 / 4,463 |
| 1 GiB / 1 files | 16 | 1.97 [1.10–2.78] | 1.56 [1.40–3.05] | 3.14 / 2.19 | 28.3 / 136.3 | 65,536 / 4,660 |
| 4 GiB / 1 files | 1 | 9.61 [9.42–10.46] | 6.02 [4.58–6.08] | 11.15 / 8.23 | 42.8 / 87.5 | 262,144 / 16,917 |
| 4 GiB / 1 files | 4 | 5.81 [5.03–5.99] | 6.07 [4.46–6.16] | 10.71 / 9.93 | 43.4 / 116.3 | 262,144 / 17,560 |
| 4 GiB / 1 files | 16 | 4.71 [4.23–7.41] | 5.83 [4.64–8.86] | 8.45 / 8.87 | 45.1 / 176.1 | 262,144 / 18,365 |
| 2 GiB / 1,000 files | 1 | 6.76 [5.75–7.38] | 2.94 [2.86–3.02] | 6.97 / 4.16 | 33.2 / 83.3 | 132,071 / 9,369 |
| 2 GiB / 1,000 files | 4 | 2.89 [2.74–4.99] | 3.13 [3.07–4.27] | 5.68 / 4.44 | 33.9 / 117.9 | 132,071 / 9,785 |
| 2 GiB / 1,000 files | 16 | 4.09 [3.40–4.10] | 3.14 [3.13–4.60] | 5.94 / 4.98 | 35.9 / 151.8 | 132,071 / 10,532 |

Positioned-write counts are from separate `strace -f -c -w` trials (all 18 qualify), include file-boundary splits, and are calls into the page cache rather than physical-device writes. Instrumented timings are excluded from the primary comparison. `/proc` generic write calls also include sockets and must not be called disk writes.

| Payload | Seeds | First verified piece visible, native / qB (s) | Shared playable condition visible, native / qB (s) |
|---|---:|---:|---:|
| 1 GiB / 1 files | 1 | 0.076 / 1.402 | 0.170 / 1.402 |
| 1 GiB / 1 files | 4 | 0.058 / 1.400 | 0.101 / 1.400 |
| 1 GiB / 1 files | 16 | 0.084 / 1.558 | 0.151 / 1.558 |
| 4 GiB / 1 files | 1 | 0.059 / 1.543 | 0.314 / 1.543 |
| 4 GiB / 1 files | 4 | 0.066 / 1.532 | 0.232 / 1.532 |
| 4 GiB / 1 files | 16 | 0.078 / 1.597 | 0.272 / 1.597 |
| 2 GiB / 1,000 files | 1 | 1.040 / 1.454 | 1.040 / 1.454 |
| 2 GiB / 1,000 files | 4 | 0.107 / 1.518 | 0.132 / 1.518 |
| 2 GiB / 1,000 files | 16 | 1.050 / 1.596 | 1.074 / 1.596 |

These are API-visible upper bounds, with a 20 ms poll plus API latency. qB piece observations often update around every 1.5 seconds, so these differences do not prove wire-level latency wins. The common playable condition is a verified first-file prefix of max(32 MiB, 3%) capped at file length, plus its final piece. The synthetic Matroska prefix decodes; a full movie watch-through is unproven. Native app-ready times and every individual observation remain in the raw results.

4 GiB recheck: native **2.034 [1.882–2.358] s**, qB **4.498 [4.495–4.517] s**, API observed. Native has lower RSS in every cell. qB wins clearly on single-seed 4 GiB and many-file downloads; several other throughput ranges overlap.

20 ms loopback netem delay in a private unprivileged network namespace, 1 GiB / 4 seeds, three alternating repeats: native **18.072 [15.889–37.634] s**, qB **5.711 [5.528–5.717] s**. All six payloads and clean drains qualify.

CPU evidence: 18 separate profiled trials qualify; 20 download/recheck profiles have nonzero samples, no collector errors and no dropped samples. Use `profile-bounded-symbols.json` / `recheck-profile-bounded-symbols.json`; earlier unbounded symbol outputs are retained but superseded. Native 4 GiB / 1 seed: 1,919 samples, hardware SHA-1 34.91%, Cx checkpoint 4.22%, scheduler execute 1.51%. Native 2 GiB / 1,000 files / 1 seed: 1,390 samples, SHA-1 27.05%, slice contains 3.24%, Cx checkpoint 2.81%, piece writer 2.59%. Stripped qB libraries lack private function attribution; the bounded analysis retains those samples as unresolved. These are exclusive user CPU samples, not inclusive stacks or wall-time causality. The positioned-write excess and latency gap were sent to Astra %7; their causes are not proven by percentages alone. The extra latency profile also qualifies: native 5.18 CPU-seconds over 40.33 wall seconds, 878 samples (SHA-1 21.75%, Cx checkpoint 2.73%); qB 2.10 CPU-seconds over 5.88 wall seconds, 352 samples. Native crossed 10% completion at 12.24 s and 50% at 30.22 s. This shows substantial waiting; the specific request/window cause remains unproven.

Public sanity: [Sintel, Blender Foundation CC-BY 3.0](https://durian.blender.org/about/), 129,302,391 bytes. Native completed in **38.641 s**, uploaded **0 bytes**; qB completed the longer attempt in **238.347 s**, uploaded **1,295 bytes**, with the identical independently verified SHA-256 `c39f5e10a35892c9d4d7e4d989407dd1fc0252c1894277e9d2b9b75a90be904d`. Both stopped cleanly. The initial qB 180 s budget failure at 107,298,679 bytes remains retained. Public payload traffic across both completions and that interrupted attempt was about 366 MB. These are sanity observations, not a speed-win claim: source availability was uncontrolled, and qB's tiny global upload cap constrained protocol requests in the controlled pilot. No further public runs were made.

Stability fixes found during comparison: `bece772` removes Watch fsync waits from its API-visible mutex; a real private Btrfs-pressure run had 2,612 API observations, worst latency 22.2 ms and a clean 6.08 s stop. `c97b8ae` reserves one ring for an ordered completion durability batch; a kernel fault control denies new ring allocation with ENOMEM while 1,000 files and their directory still sync and independently verify. The original global allocation trigger remains unproven. Earlier invalid Btrfs and tmpfs trials remain retained; they are not averaged into the green matrix.

Validation: three durability tests, five real-client/cleanup comparison controls, one stripped-ELF control, and fast gate `target/gate-fast-d7dyv48h` (710 Rust tests pass, one ignored; all 17 workflows pass; no flaky retry).

Retained evidence: `/home/oliver/Projects/odcpw/fbt-client-comparison-20261020-cod1/` (`summary-stable.json`, `stable-build.json`, `normal-stable/`, `profiles-stable/`, `syscalls-stable/`, `latency20ms-stable/`, `latency20ms-profiles/`, `public-stable/`, `public-qbt-retry/`). Each trial retains metadata, load/pressure, cgroup counters, observations and cleanup evidence. Reproduce with `head_to_head.py`; commands and isolation rules are in `README.md`.
