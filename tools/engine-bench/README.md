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
  --output target/engine-bench/baseline --runs 3 --mib 1024
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
  --output target/engine-bench/profile --runs 1 --cases download1 \
  --profile --preload /tmp/fbt-engine-alloc.so
python3 tools/engine-bench/summarize_profile.py \
  target/engine-bench/profile/download1-1/profile.json \
  --binary target/release-perf/fbittorrent --bytes 1073741824
```

Allocation counts include startup and shutdown. Diagnostic timings are never
used to claim a throughput or CPU improvement. Flat PC attribution includes an
unresolved bucket and fails on dropped samples or collection errors. Repeated
normal runs, payload integrity, zero-upload probes, piece ordering and the local
gate are required for an optimization claim.
