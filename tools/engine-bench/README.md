# Engine measurements

`fbt-bench-seed` generates deterministic random-access payloads, torrent metadata,
and 1/4/16 distinct loopback seed peers. It allocates neither the complete payload
nor a copy per seed. `run.py` reuses the regression Sandbox, API client and private
process scanner. Every daemon binds `lo`, disables upload, and uses a fresh HOME,
all XDG directories, and downloads directory. Teardown checks the ordered daemon
drain and fails if any owned process needs emergency cleanup.

The payload root must be on the filesystem being measured. `/tmp` is tmpfs on the
development machine. Do not point the runner at existing payloads or the owner's
Downloads. `--disk-root` is only a parent for newly created private directories.

Build through the repository's rch shims and shared release wrapper:

```sh
export PATH="$HOME/.rch/shims:$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS=4
flock -w 7200 /tmp/fbittorrent-release.lock systemd-run --user --scope \
  -p MemoryHigh=10G -p MemoryMax=14G --quiet -- \
  cargo build --profile release-perf --locked
flock -w 7200 /tmp/fbittorrent-release.lock systemd-run --user --scope \
  -p MemoryHigh=10G -p MemoryMax=14G --quiet -- \
  cargo build --manifest-path tools/engine-bench/Cargo.toml --release
```

Use the artifacts identified by these builds. A stale executable in `target/`
does not establish a baseline for the current source. The runner records both
the source revision and binary SHA-256. Example (adjust actual artifact paths):

```sh
python3 tools/engine-bench/run.py --binary target/release-perf/fbittorrent --binary-source BUILT_COMMIT \
  --seed tools/engine-bench/target/release/fbt-bench-seed \
  --output /path/outside/build-targets/baseline --runs 3 --mib 1024
```

The download timer includes API admission, metadata where applicable, initial
checking, peer discovery, payload receipt and verified completion. It excludes
fixture generation and the independent post-run deterministic payload comparison
and SHA-256. Streaming latency is time to a verified 8 MiB contiguous prefix;
first-piece and last-piece times are also retained. Observations poll every 20 ms
plus API latency, so sub-poll differences are not evidence of gains. Seeds record
the first distinct piece requests and, in separate `--probe-upload` runs, actively request payload while choked;
receipt of payload or an unchoke fails the run.

Hostile requests may correctly disconnect that peer; those disruption timings
are not throughput evidence. The streaming case overlays a deterministic real
Matroska fixture, measures the app's `playable.ready` transition, and decodes the
verified prefix with ffmpeg.

Every observed piece map must advertise first/last priority. A single peer also
proves first/last requests precede sequential middle requests on the wire.
Arrival order across independent seed threads is not picker order: a scheduled
peer may receive later work before the peer already assigned the first piece.

Recheck measures both admission of already-existing bytes and explicit recheck,
with paused intent preserved. The runner does not flush shared caches; actual kernel read/write-byte
counters are retained because memory pressure may evict recently written data. The 100/500-torrent cases measure
paused torrents or genuinely progressing downloads (64 MiB each, seeds capped
at 32 KiB/s per connection). Active capacity equals the requested torrent count;
queued torrents must never be presented as active. Run size, duration, repeated
observations, load, seed CPU, daemon CPU and RSS are retained in JSON.

`rw_syscalls_mb` is the kernel `/proc/PID/io` read/write syscall count per decimal
MB, **not all syscalls**. CPU seconds per GB and MB/s also use decimal units. RSS
is KiB. Peak RSS includes the process lifetime up to completion. Every successful
result must have a neighboring `teardown.json`; a measurement written before a
failing drain is not a successful benchmark.

For separate diagnostic runs on Linux/glibc x86-64, `profile.c` counts allocator
calls in a shared file, including abnormal exits. `--profile` uses the kernel's
`perf_event_open` CPU clock to sample each observed thread's user PC every 4 ms.
It needs no installed perf executable and no signal injection. Threads are
rediscovered every 25 ms; very short-lived threads can escape this profile.

Build the allocation counter with the same release wrapper:

```sh
flock -w 7200 /tmp/fbittorrent-release.lock systemd-run --user --scope \
  -p MemoryHigh=10G -p MemoryMax=14G --quiet -- \
  cc -shared -fPIC -O2 -o /tmp/fbt-engine-alloc.so tools/engine-bench/profile.c
python3 tools/engine-bench/run.py --binary target/release-perf/fbittorrent \
  --binary-source BUILT_COMMIT --seed target/release/fbt-bench-seed \
  --output /evidence/profile --runs 1 --cases download1 \
  --profile --preload /tmp/fbt-engine-alloc.so
python3 tools/engine-bench/summarize_profile.py \
  /evidence/profile/download1-1/profile.json \
  --binary target/release-perf/fbittorrent --bytes 1073741824
```

Allocation counts include startup and shutdown. Diagnostic timings are never
used to claim a throughput or CPU improvement. Flat PC attribution includes an
unresolved bucket and fails on dropped samples or collection errors. Repeated
normal runs, payload integrity, zero-upload probes, piece ordering and the local
gate are required for an optimization claim.

For syscall counts, use `--strace /absolute/path/to/strace` in a separate run.
The `-f -c -w` report includes the daemon's owned helper execs. Its time column
sums wall-clock syscall durations across threads, so futex wait time is **not**
CPU attribution. Divide the total call count by payload bytes / 1e6.

The seed SHA-256 is recorded as well as the daemon's. Rch can return a different
artifact directory when it changes workers or falls back locally: take the
artifact from the actual build output, never infer it from an old target file.

Harness controls (no engine performance claim):

```sh
cargo test --manifest-path tools/engine-bench/Cargo.toml --locked
python3 tools/engine-bench/seed_test.py /actual/build/path/fbt-bench-seed
python3 tools/engine-bench/cleanup_test.py
```

The cleanup control injects both a setup exception and KeyboardInterrupt after
starting a private child. Both must fail qualification and reap the child.
Low-rate active workloads accept transient `stalled` gauges between blocks, but
require every torrent to make verified byte progress in the measured window.
Paused and queued torrents cannot pass as active.

Keep retained binaries and measurement outputs **outside all Cargo target
directories**. Artifact retrieval or build-cache cleanup can replace target
contents. Use a dedicated evidence directory, with its binary copies identified
by SHA-256. Do not retain the only copy of a baseline inside `target/`.

For a comparison, alternate the before/after binaries in each repeat (AB then
BA), using the same seed, workload and private filesystem. Warmup cases run all
behaviour checks but live under `warmup-*` names and must not enter statistics:

```sh
python3 tools/engine-bench/compare.py --before /evidence/bin/control \
  --after /evidence/bin/candidate --before-source CONTROL_REV \
  --after-source CANDIDATE_REV_OR_PATCH --seed /evidence/bin/fbt-bench-seed \
  --output /evidence/paired --disk-root /tmp --runs 10 --warmup 3
```

A completed/dormant API view may report unknown upload counters. Wire-observed
payload bytes and Unchoke messages must still be zero, and every known API
counter must be zero. This is independent of prose and optional field presence.

For the shutdown regression on a real Btrfs disk, run three verified 1 GiB/four-peer
transfers alongside two private direct-I/O writers. The writers reuse 512 MiB in
total, use private XDG paths, and stop before their isolation is removed. Any
emergency cleanup fails qualification; `teardown.json` records total shutdown
milliseconds, in addition to the engine's drain diagnostic:

```sh
python3 tools/engine-bench/shutdown_test.py --binary /evidence/bin/fbittorrent \
  --binary-source BUILT_COMMIT --seed /evidence/bin/fbt-bench-seed \
  --output /evidence/shutdown-pressure --disk-root /path/on/btrfs
```

## Installed-client comparison

`head_to_head.py` compares a retained fbittorrent binary with the installed
qBittorrent-nox 5.2 client using private HOME/XDG paths and a new qBittorrent
profile. Its WebUI binds localhost, and listener PID ownership is checked before
any settings mutation. The local matrix is 1/4 GiB single-file and 2 GiB across
1,000 files, each with 1/4/16 distinct seed endpoints. Both use first/last plus
sequential selection; the qBittorrent private profile permits multiple peers
from one IP and limits upload to 1 KiB/s. Native upload remains off.

```sh
flock -w 7200 /tmp/fbittorrent-release.lock systemd-run --user --scope \
  -p MemoryHigh=10G -p MemoryMax=14G --quiet -- \
  python3 tools/engine-bench/head_to_head.py --binary /evidence/bin/fbittorrent \
  --binary-source REV_OR_PATCH --seed /evidence/bin/fbt-bench-seed \
  --output /evidence/client-normal --runs 3
python3 tools/engine-bench/summarize_clients.py /evidence/client-normal
```

Trials alternate client order. Independent post-timer verification hashes every
piece across file boundaries and compares whole-payload SHA-256 between clients.
The shared playable byte condition is the first file's verified 3%/32 MiB prefix
(or the complete file if smaller), plus its last piece; the prefix must decode.
Native `app_ready_s` is separate. API polling adds at least 20 ms uncertainty.
`/proc` CPU/RSS/write-call counters describe the client process and all its
threads; write calls include sockets. Use separate `--strace /path/to/strace`
trials for positioned disk-write call counts, and `--profile` for per-phase CPU
samples. Instrumented timings never enter the comparison summary.

Use `--public-torrent /path/to/legal.torrent` for one back-to-back sanity pair:
the tool refuses payloads above 200 MiB and limits downloads to 4 MiB/s. Public
swarm timings are not a controlled comparison. `--pilot-mib 64` exercises the
harness at small scale and is rejected by the normal summary. Every benchmark,
including pilots and diagnostics, uses the lock and scope wrapper above.

Focused controls (tiny transfers, no timing qualification):

```sh
FBT_BENCH_BINARY=/evidence/bin/fbittorrent FBT_BENCH_SEED=/evidence/bin/fbt-bench-seed \
  python3 tools/engine-bench/head_to_head_test.py
python3 tools/engine-bench/seed_test.py /evidence/bin/fbt-bench-seed --multi
```
